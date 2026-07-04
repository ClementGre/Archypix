import {useCallback, useEffect, useRef, useState} from 'react'
import {AlertCircle, ArchiveRestore, Check, CloudUpload, FileImage, Loader2, RotateCw, Tag as TagIcon, X} from 'lucide-react'
import {toast} from 'sonner'
import {useQueryClient} from '@tanstack/react-query'
import {Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {TagPicker} from '@/components/tags/TagPicker'
import {beginUploadBatch, completeUpload, restorePicture} from '@/api/pictures'
import {invalidatePictures, invalidatePicturesAndTags, invalidateStorageDebounced} from '@/lib/invalidation'
import {cn, TagPath} from '@/lib/utils'
import {filesFromDataTransfer, isHiddenFile} from '@/lib/uploadFiles'
import {apiErrorMessage} from '@/api/client'
import {useStorage} from '@/hooks/useSettings'

// ── Types ────────────────────────────────────────────────────────────────────

type UploadStatus = 'pending' | 'preparing' | 'uploading' | 'completing' | 'done' | 'deduplicated' | 'error'
type Phase = 'idle' | 'uploading' | 'complete'

interface UploadItem {
    key: string
    file: File
    pictureId: string
    presignedUrl: string
    /** SHA-256 (lowercase hex), computed lazily when the slot is about to upload. Kept so a retry
     *  reuses it (only the presign is re-requested). */
    hash?: string
    status: UploadStatus
    progress: number
    error?: string
    /** Dedup hit on a trashed picture — surfaced so the user can restore them (feature 15). */
    wasDeleted?: boolean
}

// How many slots run concurrently. Each slot hashes (buffers the whole file in memory), re-presigns
// just-in-time, then uploads — a small bound keeps a 1k-photo batch from reading every file at once
// (which black-screened phones).
const UPLOAD_CONCURRENCY = 4

// ── Helpers ──────────────────────────────────────────────────────────────────

function fileKey(f: File): string {
    return `${f.name}:${f.size}:${f.lastModified}`
}

/**
 * The per-batch import label (`Uploaded_YYYY_MM_DD_HH_MM`, local time) — fixed by the front so the
 * whole batch shares one date (not the presign/complete time). The backend tags new uploads with it
 * and already-existing duplicates with `<label>.AlreadyExisting[.Deleted]` (feature 15).
 */
function makeUploadLabel(d = new Date()): string {
    const p = (n: number) => String(n).padStart(2, '0')
    return `Uploaded.${d.getFullYear()}_${p(d.getMonth() + 1)}_${p(d.getDate())}_${p(d.getHours())}_${p(d.getMinutes())}`
}

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
}

/** A friendly over-quota message for a 413/507 (feature 22); otherwise the server's error text. */
function uploadErrorMessage(e: unknown): string {
    const status = (e as { response?: { status?: number } })?.response?.status
    if (status === 413 || status === 507) {
        return 'Not enough storage space — this would exceed your quota.'
    }
    return apiErrorMessage(e)
}

/**
 * SHA-256 of the file as lowercase hex — byte-identical to the backend worker's
 * `archypix-common::hash::hash_file` (plain SHA-256 over the full file). Sent on upload completion
 * as a provisional ETag/dedupe key. Buffers the whole file in memory (fine for photos).
 */
async function sha256Hex(file: File): Promise<string> {
    const buf = await file.arrayBuffer()
    const digest = await crypto.subtle.digest('SHA-256', buf)
    return Array.from(new Uint8Array(digest))
        .map((b) => b.toString(16).padStart(2, '0'))
        .join('')
}

function uploadToS3(url: string, file: File, onProgress: (pct: number) => void): Promise<void> {
    return new Promise((resolve, reject) => {
        const xhr = new XMLHttpRequest()
        xhr.upload.onprogress = (e) => {
            if (e.lengthComputable) onProgress(Math.round((e.loaded / e.total) * 95))
        }
        xhr.onload = () => {
            if (xhr.status >= 200 && xhr.status < 300) resolve()
            else reject(new Error(`S3 upload failed: ${xhr.status}`))
        }
        xhr.onerror = () => reject(new Error('Network error during upload'))
        xhr.open('PUT', url)
        xhr.setRequestHeader('Content-Type', file.type || 'application/octet-stream')
        xhr.send(file)
    })
}

// ── Sub-components ───────────────────────────────────────────────────────────

/**
 * Thumbnail that only materialises its object URL once the row scrolls near the viewport —
 * creating 1k object URLs + <img> decodes up front froze phones in big batches.
 */
function FilePreview({file}: { file: File }) {
    const [url, setUrl] = useState<string | null>(null)
    const ref = useRef<HTMLDivElement>(null)
    useEffect(() => {
        if (!file.type.startsWith('image/')) return
        const el = ref.current
        if (!el) return
        let objectUrl: string | null = null
        const io = new IntersectionObserver(
            (entries) => {
                if (entries[0]?.isIntersecting) {
                    objectUrl = URL.createObjectURL(file)
                    setUrl(objectUrl)
                    io.disconnect()
                }
            },
            {rootMargin: '200px'},
        )
        io.observe(el)
        return () => {
            io.disconnect()
            if (objectUrl) URL.revokeObjectURL(objectUrl)
        }
    }, [file])

    return (
        <div ref={ref} className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded bg-muted">
            {url ? (
                <img src={url} alt="" className="h-10 w-10 object-cover"/>
            ) : (
                <FileImage className="h-5 w-5 text-muted-foreground"/>
            )}
        </div>
    )
}

function StatusIcon({status}: { status: UploadStatus }) {
    if (status === 'done') return <Check className="h-4 w-4 shrink-0 text-emerald-500"/>
    // A dedup hit: the file already existed, so it was not re-uploaded — orange check.
    if (status === 'deduplicated') return <Check className="h-4 w-4 shrink-0 text-amber-500"/>
    if (status === 'error') return <AlertCircle className="h-4 w-4 shrink-0 text-destructive"/>
    if (status === 'preparing' || status === 'uploading' || status === 'completing')
        return <Loader2 className="h-4 w-4 shrink-0 animate-spin text-primary"/>
    return <div className="h-4 w-4 shrink-0"/>
}

/** A slim progress bar (0–100). */
function ProgressBar({pct, className}: { pct: number; className?: string }) {
    return (
        <div className="h-2 w-full overflow-hidden rounded-full bg-muted">
            <div
                className={cn('h-full rounded-full bg-primary transition-all duration-200', className)}
                style={{width: `${Math.max(0, Math.min(100, pct))}%`}}
            />
        </div>
    )
}

// ── Main component ───────────────────────────────────────────────────────────

export interface UploadDialogProps {
    open: boolean
    onOpenChange: (open: boolean) => void
    initialFiles?: File[]
}

export function UploadDialog({open, onOpenChange, initialFiles}: UploadDialogProps) {
    const queryClient = useQueryClient()
    const {data: storage} = useStorage()
    const fileInputRef = useRef<HTMLInputElement>(null)
    const folderInputRef = useRef<HTMLInputElement>(null)
    const dropzoneRef = useRef<HTMLDivElement>(null)

    const [files, setFiles] = useState<File[]>([])
    const [tags, setTags] = useState<string[]>([])
    const [phase, setPhase] = useState<Phase>('idle')
    const [items, setItems] = useState<UploadItem[]>([])
    const [dropActive, setDropActive] = useState(false)
    const [restoring, setRestoring] = useState(false)
    const [restored, setRestored] = useState(false)
    const firstSuccess = useRef(false)
    // The import label for the in-flight batch — fixed once at upload start so retries reuse it.
    const uploadLabel = useRef('')

    // Seed files from parent (gallery drop zone)
    useEffect(() => {
        if (!open || !initialFiles?.length) return
        setFiles((prev) => {
            const existing = new Set(prev.map(fileKey))
            return [...prev, ...initialFiles.filter((f) => !existing.has(fileKey(f)))]
        })
    }, [open, initialFiles])

    // Reset everything when closed
    useEffect(() => {
        if (!open) {
            setFiles([])
            setTags([])
            setPhase('idle')
            setItems([])
            setRestored(false)
        }
    }, [open])

    const addFiles = useCallback((newFiles: FileList | File[]) => {
        const arr = Array.from(newFiles)
        setFiles((prev) => {
            const existing = new Set(prev.map(fileKey))
            return [...prev, ...arr.filter((f) => !existing.has(fileKey(f)))]
        })
    }, [])

    // Drag-over handling inside the dialog dropzone
    const onDragOver = (e: React.DragEvent) => {
        e.preventDefault()
        if (e.dataTransfer.types.includes('Files')) setDropActive(true)
    }
    const onDragLeave = (e: React.DragEvent) => {
        if (!dropzoneRef.current?.contains(e.relatedTarget as Node)) setDropActive(false)
    }
    const onDrop = (e: React.DragEvent) => {
        e.preventDefault()
        setDropActive(false)
        // Recurse into dropped directories (excluding hidden files) — must read the entries here,
        // synchronously, before the DataTransfer is cleared.
        void filesFromDataTransfer(e.dataTransfer).then((fs) => {
            if (fs.length) addFiles(fs)
        })
    }

    // Patch by the per-file `key`, not `pictureId`: an in-batch duplicate shares the first file's
    // picture id, so keying on picture id would let one file's progress overwrite the other.
    const patchItem = useCallback((key: string, patch: Partial<UploadItem>) => {
        setItems((prev) => prev.map((it) => (it.key === key ? {...it, ...patch} : it)))
    }, [])

    // Run one slot end-to-end: hash (lazily, reused on retry) → fresh presign → S3 PUT + complete.
    // The presign is requested **just before** the upload so it can't expire while earlier files in
    // the batch are still transferring, and a retry always mints a brand-new URL.
    const processItem = useCallback(
        async (item: UploadItem): Promise<boolean> => {
            const {key, file} = item
            patchItem(key, {status: 'preparing', progress: 0, error: undefined})

            // Hash the file (reuse a value computed on an earlier attempt; only the presign is fresh).
            let hash = item.hash
            if (!hash) {
                hash = await sha256Hex(file).catch(() => undefined)
                patchItem(key, {hash})
            }

            // Fresh presign for this single file: carries the hash (upload-time dedup) + tags (assigned
            // to a dedup hit) + the import label (tags duplicates AlreadyExisting[.Deleted]).
            let slot: Awaited<ReturnType<typeof beginUploadBatch>>[number] | undefined
            try {
                slot = (
                    await beginUploadBatch(
                        [{filename: file.name, file_hash: hash, size: file.size}],
                        tags.length ? tags : undefined,
                        uploadLabel.current || undefined,
                    )
                )[0]
            } catch (e) {
                patchItem(key, {status: 'error', error: uploadErrorMessage(e)})
                return false
            }
            if (!slot) {
                patchItem(key, {status: 'error', error: 'No upload slot returned'})
                return false
            }
            if (slot.duplicate) {
                // Already in the library — settled server-side (tags assigned); nothing to upload.
                patchItem(key, {
                    status: 'deduplicated',
                    pictureId: slot.picture_id,
                    wasDeleted: slot.was_deleted,
                    progress: 100,
                })
                return true
            }

            const pictureId = slot.picture_id
            const presignedUrl = slot.presigned_url ?? ''
            patchItem(key, {status: 'uploading', pictureId, presignedUrl, progress: 0})
            try {
                await uploadToS3(presignedUrl, file, (pct) => patchItem(key, {progress: pct}))
                patchItem(key, {status: 'completing', progress: 97})
                await completeUpload(pictureId, {
                    mime_type: file.type || undefined,
                    file_size: file.size,
                    file_hash: hash,
                    initial_tags: tags.length ? tags : undefined,
                    upload_label: uploadLabel.current || undefined,
                })
                patchItem(key, {status: 'done', progress: 100})
                if (!firstSuccess.current) {
                    firstSuccess.current = true
                    invalidatePictures(queryClient)
                }
                return true
            } catch (e) {
                patchItem(key, {status: 'error', error: uploadErrorMessage(e)})
                return false
            }
        },
        [patchItem, tags, queryClient],
    )

    // Drain a list of slots through bounded-concurrency workers.
    const runQueue = useCallback(
        async (queue: UploadItem[]) => {
            const work = [...queue]

            async function worker() {
                while (true) {
                    const item = work.shift()
                    if (!item) break
                    await processItem(item)
                }
            }

            await Promise.all(Array.from({length: Math.min(UPLOAD_CONCURRENCY, work.length)}, worker))
        },
        [processItem],
    )

    // Uploads can dedup onto existing pictures (re-tagging them) and trigger background re-tagging,
    // so refresh pictures + tags broadly — `['tags']` also refreshes per-picture tag caches (e.g. an
    // already-existing picture open in the sidebar), which the old narrow keys missed.
    const invalidateAll = useCallback(() => {
        invalidatePicturesAndTags(queryClient)
        // Uploads change billed usage → refresh the storage bar/breakdown (feature 22), debounced
        // since this fires once per file/batch during a multi-file upload.
        invalidateStorageDebounced(queryClient)
    }, [queryClient])

    async function startUpload() {
        if (!files.length || phase !== 'idle') return

        firstSuccess.current = false
        uploadLabel.current = makeUploadLabel()

        const initialItems: UploadItem[] = files.map((file) => ({
            key: fileKey(file),
            file,
            pictureId: '',
            presignedUrl: '',
            status: 'pending',
            progress: 0,
        }))
        setItems(initialItems)
        setPhase('uploading')

        await runQueue(initialItems)

        setPhase('complete')
        // The backend debounces the pipeline wake, so per-file completions coalesce on their own;
        // refresh pictures + tags (uploads may pick up pipeline tags), then again once converged.
        invalidateAll()
    }

    // Retry a batch of failed items (single retry passes a one-element list). Reset them to `pending`
    // first so the error count drops to 0 and the retry buttons hide — it really looks like the
    // upload restarted with only the errored files. Each retried slot re-presigns (fresh URL).
    const retryItems = useCallback(
        async (toRetry: UploadItem[]) => {
            if (!toRetry.length) return
            const keys = new Set(toRetry.map((i) => i.key))
            setItems((prev) =>
                prev.map((it) =>
                    keys.has(it.key) ? {...it, status: 'pending', progress: 0, error: undefined} : it,
                ),
            )
            setPhase('uploading')
            await runQueue(toRetry)
            setPhase('complete')
            invalidateAll()
        },
        [runQueue, invalidateAll],
    )

    function resetForMore() {
        setFiles([])
        setItems([])
        setPhase('idle')
    }

    // Derived counts
    const doneCount = items.filter((i) => i.status === 'done').length
    const dedupCount = items.filter((i) => i.status === 'deduplicated').length
    const errorCount = items.filter((i) => i.status === 'error').length
    const failedItems = items.filter((i) => i.status === 'error')
    // Already-existing duplicates, split into live vs trashed (the latter can be restored).
    const existingCount = items.filter((i) => i.status === 'deduplicated' && !i.wasDeleted).length
    const deletedItems = items.filter((i) => i.status === 'deduplicated' && i.wasDeleted)
    const isBusy = phase === 'uploading'
    const settledCount = doneCount + dedupCount + errorCount
    const label = uploadLabel.current.replaceAll('.', '/')

    // Storage preflight (feature 22): the selected bytes vs remaining quota. `available` is null when
    // unlimited. Duplicates may not actually consume space, so this is an upper bound — a conservative
    // warning, not a hard client-side block beyond the "quota full" case.
    const selectedBytes = files.reduce((sum, f) => sum + f.size, 0)
    const available = storage?.available_bytes ?? null
    const quotaFull = storage?.warn_level === 'full'
    const projectedOver = available !== null && selectedBytes > available
    const uploadBlocked = phase === 'idle' && files.length > 0 && quotaFull

    const undeleteExisting = async () => {
        if (!deletedItems.length || restoring) return
        setRestoring(true)
        try {
            await Promise.all(deletedItems.map((it) => restorePicture(it.pictureId)))
            setRestored(true)
            invalidateAll()
        } catch (e) {
            toast.error(apiErrorMessage(e))
        } finally {
            setRestoring(false)
        }
    }

    // Overall advancement: average of each slot's progress (settled = 100, in-flight = its S3 %),
    // floored to one decimal so a near-complete batch reads e.g. 99.5% rather than jumping to 100.
    const overallPct = items.length
        ? Math.floor(
        (items.reduce((sum, it) => {
                if (it.status === 'done' || it.status === 'deduplicated' || it.status === 'error')
                    return sum + 100
                if (it.status === 'uploading' || it.status === 'completing') return sum + it.progress
                return sum
            }, 0) /
            (items.length * 100)) *
        1000,
    ) / 10
        : 0

    return (
        <Dialog
            open={open}
            onOpenChange={(o) => {
                if (!o && isBusy) return
                onOpenChange(o)
            }}
        >
            <DialogContent
                className="flex max-h-[90vh] max-w-2xl flex-col gap-0 overflow-hidden p-0"
                onInteractOutside={(e) => {
                    if (isBusy) e.preventDefault()
                }}
            >
                <DialogHeader className="px-6 pt-6 pb-4">
                    <DialogTitle>Upload photos</DialogTitle>
                </DialogHeader>

                {/* ── Overall progress (outside the scrollable list, always visible) ── */}
                {phase !== 'idle' && (
                    <div className="space-y-1.5 border-b border-border px-6 pb-4">
                        <div className="flex items-center justify-between text-sm">
                            <span className="text-muted-foreground">
                                {phase === 'uploading' &&
                                    (settledCount < items.length
                                        ? `Uploading… ${settledCount} of ${items.length}`
                                        : 'Finishing up…')}
                                {phase === 'complete' && (
                                    errorCount === 0 ? (
                                        <span className="text-emerald-500">
                                            {doneCount} photo{doneCount !== 1 ? 's' : ''} uploaded
                                        </span>
                                    ) : (
                                        <span>
                                            {doneCount} uploaded
                                            <span className="text-destructive"> · {errorCount} failed</span>
                                        </span>
                                    )
                                )}
                                {phase === 'complete' && dedupCount > 0 && (
                                    <span className="text-amber-500"> · {dedupCount} already existed</span>
                                )}
                            </span>
                            <span className="tabular-nums text-xs text-muted-foreground">{overallPct.toFixed(1)}%</span>
                        </div>
                        <ProgressBar pct={overallPct}/>
                        {phase === 'complete' && errorCount > 0 && (
                            <div className="pt-1">
                                <Button
                                    variant="outline"
                                    size="sm"
                                    className="h-7 gap-1.5 text-xs"
                                    onClick={() => retryItems(failedItems)}
                                >
                                    <RotateCw className="h-3.5 w-3.5"/>
                                    Retry {errorCount} failed
                                </Button>
                            </div>
                        )}

                        {/* Import summary — what was tagged with the batch label (feature 15). */}
                        {phase === 'complete' && (doneCount > 0 || dedupCount > 0) && (
                            <div className="mt-2 space-y-1 rounded-md border border-border bg-muted/30 px-3 py-2 text-xs">
                                {label && (
                                    <p className="text-muted-foreground">
                                        Tagged this import <span className="font-medium text-foreground">{TagPath.toDisplay(label)}</span>
                                    </p>
                                )}
                                <ul className="space-y-0.5 text-muted-foreground">
                                    {doneCount > 0 && (
                                        <li>· {doneCount} uploaded → <span className="font-mono">{label}</span></li>
                                    )}
                                    {existingCount > 0 && (
                                        <li>· {existingCount} already existed → <span className="font-mono">{label}/AlreadyExisting</span></li>
                                    )}
                                    {deletedItems.length > 0 && (
                                        <li>· {deletedItems.length} of those were in the trash → <span className="font-mono">{label}/AlreadyExisting/Deleted</span></li>
                                    )}
                                    <li className="text-foreground">· {doneCount + dedupCount} total</li>
                                </ul>
                                {deletedItems.length > 0 && (
                                    restored ? (
                                        <p className="flex items-center gap-1 text-emerald-500">
                                            <Check className="h-3.5 w-3.5"/>
                                            Restored {deletedItems.length} from trash
                                        </p>
                                    ) : (
                                        <Button
                                            variant="outline"
                                            size="sm"
                                            className="mt-1 h-7 gap-1.5 text-xs"
                                            disabled={restoring}
                                            onClick={undeleteExisting}
                                        >
                                            {restoring ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <ArchiveRestore className="h-3.5 w-3.5"/>}
                                            Restore {deletedItems.length} deleted from trash
                                        </Button>
                                    )
                                )}
                            </div>
                        )}
                    </div>
                )}

                <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 py-4">
                    {/* ── Dropzone / file list ─────────────────────────────── */}
                    <div
                        ref={dropzoneRef}
                        onDragOver={phase === 'idle' ? onDragOver : undefined}
                        onDragLeave={phase === 'idle' ? onDragLeave : undefined}
                        onDrop={phase === 'idle' ? onDrop : undefined}
                    >
                        {phase === 'idle' && files.length === 0 && (
                            <>
                                <button
                                    type="button"
                                    className={cn(
                                        'flex w-full flex-col items-center justify-center gap-3 rounded-lg border-2 border-dashed py-12 transition-colors',
                                        dropActive
                                            ? 'border-primary bg-primary/5'
                                            : 'border-border hover:border-primary/40 hover:bg-muted/40',
                                    )}
                                    onClick={() => fileInputRef.current?.click()}
                                >
                                    <CloudUpload
                                        className={cn(
                                            'h-10 w-10 transition-colors',
                                            dropActive ? 'text-primary' : 'text-muted-foreground',
                                        )}
                                    />
                                    <div className="text-center">
                                        <p className="text-sm font-medium">Drop photos or folders here</p>
                                        <p className="mt-0.5 text-xs text-muted-foreground">
                                            or click to browse your files
                                        </p>
                                    </div>
                                </button>
                                <div className="mt-2 text-center">
                                    <button
                                        type="button"
                                        className="text-xs text-primary hover:underline"
                                        onClick={() => folderInputRef.current?.click()}
                                    >
                                        or select an entire folder
                                    </button>
                                </div>
                            </>
                        )}

                        {/* File list — idle phase */}
                        {phase === 'idle' && files.length > 0 && (
                            <div
                                className={cn(
                                    'rounded-lg border transition-colors',
                                    dropActive ? 'border-primary bg-primary/5' : 'border-border',
                                )}
                            >
                                <div className="divide-y divide-border">
                                    {files.map((file) => (
                                        <div key={fileKey(file)} className="flex items-center gap-3 px-3 py-2.5">
                                            <FilePreview file={file}/>
                                            <div className="min-w-0 flex-1">
                                                <p className="truncate text-sm">{file.name}</p>
                                                <p className="text-xs text-muted-foreground">
                                                    {formatBytes(file.size)}
                                                </p>
                                            </div>
                                            <Button
                                                variant="ghost"
                                                size="icon"
                                                className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                                                onClick={() =>
                                                    setFiles((prev) => prev.filter((f) => fileKey(f) !== fileKey(file)))
                                                }
                                            >
                                                <X className="h-3.5 w-3.5"/>
                                            </Button>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        )}

                        {/* File list — uploading / complete phase */}
                        {phase !== 'idle' && items.length > 0 && (
                            <div className="rounded-lg border border-border">
                                <div className="divide-y divide-border">
                                    {items.map((item) => (
                                        <div key={item.key} className="flex items-center gap-3 px-3 py-2.5">
                                            <FilePreview file={item.file}/>
                                            <div className="min-w-0 flex-1">
                                                <p className="truncate text-sm">{item.file.name}</p>
                                                {item.error ? (
                                                    <p className="truncate text-xs text-destructive">
                                                        {item.error}
                                                    </p>
                                                ) : item.status === 'deduplicated' ? (
                                                    <p className="truncate text-xs text-amber-500">
                                                        Already in your library
                                                    </p>
                                                ) : item.status === 'preparing' ? (
                                                    <p className="text-xs text-muted-foreground">Preparing…</p>
                                                ) : (
                                                    <p className="text-xs text-muted-foreground">
                                                        {formatBytes(item.file.size)}
                                                    </p>
                                                )}
                                                {(item.status === 'uploading' ||
                                                    item.status === 'completing') && (
                                                    <div className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-muted">
                                                        <div
                                                            className="h-full rounded-full bg-primary transition-all duration-200"
                                                            style={{width: `${item.progress}%`}}
                                                        />
                                                    </div>
                                                )}
                                            </div>
                                            {item.status === 'error' && (
                                                <Button
                                                    variant="ghost"
                                                    size="icon"
                                                    className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                                                    title="Retry"
                                                    onClick={() => retryItems([item])}
                                                >
                                                    <RotateCw className="h-3.5 w-3.5"/>
                                                </Button>
                                            )}
                                            <StatusIcon status={item.status}/>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        )}
                    </div>

                    {/* ── Add more (idle only) ─────────────────────────────── */}
                    {phase === 'idle' && files.length > 0 && (
                        <div className="flex items-center gap-3">
                            <button
                                type="button"
                                className="text-xs text-primary hover:underline"
                                onClick={() => fileInputRef.current?.click()}
                            >
                                + Add more photos
                            </button>
                            <button
                                type="button"
                                className="text-xs text-primary hover:underline"
                                onClick={() => folderInputRef.current?.click()}
                            >
                                + Add folder
                            </button>
                            <span className="text-xs text-muted-foreground">
                                {files.length} file{files.length !== 1 ? 's' : ''} selected
                            </span>
                        </div>
                    )}

                    {/* ── Tag section (idle only) ──────────────────────────── */}
                    {phase === 'idle' && (
                        <div className="space-y-2 pb-1">
                            <div className="flex items-center gap-1.5">
                                <TagIcon className="h-3.5 w-3.5 text-muted-foreground"/>
                                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                                    Apply tags to all
                                </span>
                            </div>
                            <div className="flex flex-wrap items-center gap-1.5">
                                {tags.map((t) => (
                                    <Badge
                                        key={t}
                                        variant="secondary"
                                        className="gap-1 pr-1 text-xs font-normal"
                                    >
                                        {TagPath.toDisplay(t)}
                                        <button
                                            type="button"
                                            className="ml-0.5 rounded opacity-60 transition-opacity hover:opacity-100"
                                            onClick={() =>
                                                setTags((prev) => prev.filter((x) => x !== t))
                                            }
                                        >
                                            <X className="h-3 w-3"/>
                                        </button>
                                    </Badge>
                                ))}
                                <TagPicker
                                    onSelect={(wire) =>
                                        setTags((prev) =>
                                            prev.includes(wire) ? prev : [...prev, wire],
                                        )
                                    }
                                    excludePaths={tags}
                                    triggerLabel="Add tag"
                                />
                            </div>
                        </div>
                    )}

                    {/* ── Storage preflight (feature 22) ───────────────────── */}
                    {phase === 'idle' && storage && (storage.quota_bytes ?? 0) > 0 && (
                        <div
                            className={cn(
                                'rounded-md border px-3 py-2 text-xs',
                                quotaFull || projectedOver
                                    ? 'border-destructive/40 bg-destructive/5 text-destructive'
                                    : storage.warn_level === 'critical' || storage.warn_level === 'warn'
                                        ? 'border-amber-500/40 bg-amber-500/5 text-amber-600 dark:text-amber-500'
                                        : 'border-border bg-muted/30 text-muted-foreground',
                            )}
                        >
                            {quotaFull ? (
                                <span>Storage is full. Free up space or empty your trash before uploading.</span>
                            ) : projectedOver ? (
                                <span>
                                    Selected {formatBytes(selectedBytes)} exceeds your remaining{' '}
                                    {formatBytes(available ?? 0)} — some files may fail. Free up space first.
                                </span>
                            ) : (
                                <span>
                                    {formatBytes(available ?? 0)} free of {formatBytes(storage.quota_bytes ?? 0)}
                                    {files.length > 0 && <> · selected {formatBytes(selectedBytes)}</>}
                                </span>
                            )}
                        </div>
                    )}
                </div>

                {/* ── Footer ──────────────────────────────────────────────── */}
                <DialogFooter className="border-t border-border px-6 py-4">
                    {phase === 'complete' ? (
                        <>
                            <Button variant="outline" onClick={resetForMore}>
                                Upload more
                            </Button>
                            <Button onClick={() => onOpenChange(false)}>Close</Button>
                        </>
                    ) : (
                        <>
                            <Button
                                variant="outline"
                                onClick={() => onOpenChange(false)}
                                disabled={isBusy}
                            >
                                Cancel
                            </Button>
                            <Button
                                onClick={startUpload}
                                disabled={files.length === 0 || isBusy || uploadBlocked}
                                title={uploadBlocked ? 'Storage is full' : undefined}
                            >
                                {isBusy ? (
                                    <>
                                        <Loader2 className="mr-2 h-4 w-4 animate-spin"/>
                                        Uploading…
                                    </>
                                ) : uploadBlocked ? (
                                    <>
                                        <AlertCircle className="mr-2 h-4 w-4"/>
                                        Storage full
                                    </>
                                ) : (
                                    <>
                                        <CloudUpload className="mr-2 h-4 w-4"/>
                                        {files.length > 0
                                            ? `Upload ${files.length} photo${files.length !== 1 ? 's' : ''}`
                                            : 'Upload photos'}
                                    </>
                                )}
                            </Button>
                        </>
                    )}
                </DialogFooter>
            </DialogContent>

            <input
                ref={fileInputRef}
                type="file"
                multiple
                accept="image/*,video/*"
                className="hidden"
                onChange={(e) => {
                    if (e.target.files) {
                        addFiles(Array.from(e.target.files).filter((f) => !isHiddenFile(f)))
                        e.target.value = ''
                    }
                }}
            />
            {/* Folder picker: `webkitdirectory` selects a directory; we take every file recursively,
                excluding hidden ones (dotfiles / dot-dirs). Attributes are set imperatively since
                they aren't typed on React's input props. */}
            <input
                ref={(el) => {
                    folderInputRef.current = el
                    if (el) {
                        el.setAttribute('webkitdirectory', '')
                        el.setAttribute('directory', '')
                    }
                }}
                type="file"
                multiple
                className="hidden"
                onChange={(e) => {
                    if (e.target.files) {
                        addFiles(Array.from(e.target.files).filter((f) => !isHiddenFile(f)))
                        e.target.value = ''
                    }
                }}
            />
        </Dialog>
    )
}
