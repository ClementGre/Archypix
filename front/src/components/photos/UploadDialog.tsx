import {useCallback, useEffect, useRef, useState} from 'react'
import {AlertCircle, Check, CloudUpload, FileImage, Loader2, Tag as TagIcon, X} from 'lucide-react'
import {toast} from 'sonner'
import {useQueryClient} from '@tanstack/react-query'
import {Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {TagPicker} from '@/components/tags/TagPicker'
import {beginUploadBatch, completeUpload} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {cn, TagPath} from '@/lib/utils'
import {apiErrorMessage} from '@/api/client'

// ── Types ────────────────────────────────────────────────────────────────────

type UploadStatus = 'pending' | 'uploading' | 'completing' | 'done' | 'error'
type Phase = 'idle' | 'uploading' | 'complete'

interface UploadItem {
    key: string
    file: File
    pictureId: string
    presignedUrl: string
    status: UploadStatus
    progress: number
    error?: string
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function fileKey(f: File): string {
    return `${f.name}:${f.size}:${f.lastModified}`
}

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`
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

function FilePreview({file}: { file: File }) {
    const [url, setUrl] = useState<string | null>(null)
    useEffect(() => {
        if (!file.type.startsWith('image/')) return
        const u = URL.createObjectURL(file)
        setUrl(u)
        return () => URL.revokeObjectURL(u)
    }, [file])

    if (!url) {
        return (
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded bg-muted">
                <FileImage className="h-5 w-5 text-muted-foreground"/>
            </div>
        )
    }
    return <img src={url} alt="" className="h-10 w-10 shrink-0 rounded object-cover"/>
}

function StatusIcon({status}: { status: UploadStatus }) {
    if (status === 'done') return <Check className="h-4 w-4 shrink-0 text-emerald-500"/>
    if (status === 'error') return <AlertCircle className="h-4 w-4 shrink-0 text-destructive"/>
    if (status === 'uploading' || status === 'completing')
        return <Loader2 className="h-4 w-4 shrink-0 animate-spin text-primary"/>
    return <div className="h-4 w-4 shrink-0"/>
}

// ── Main component ───────────────────────────────────────────────────────────

export interface UploadDialogProps {
    open: boolean
    onOpenChange: (open: boolean) => void
    initialFiles?: File[]
}

export function UploadDialog({open, onOpenChange, initialFiles}: UploadDialogProps) {
    const queryClient = useQueryClient()
    const fileInputRef = useRef<HTMLInputElement>(null)
    const dropzoneRef = useRef<HTMLDivElement>(null)

    const [files, setFiles] = useState<File[]>([])
    const [tags, setTags] = useState<string[]>([])
    const [phase, setPhase] = useState<Phase>('idle')
    const [items, setItems] = useState<UploadItem[]>([])
    const [dropActive, setDropActive] = useState(false)

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
        if (e.dataTransfer.files.length > 0) addFiles(e.dataTransfer.files)
    }

    function patchItem(pictureId: string, patch: Partial<UploadItem>) {
        setItems((prev) => prev.map((it) => (it.pictureId === pictureId ? {...it, ...patch} : it)))
    }

    async function startUpload() {
        if (!files.length || phase !== 'idle') return

        // Batch-presign all files at once
        let slots: Array<{ picture_id: string; presigned_url: string }>
        try {
            slots = await beginUploadBatch(files.map((f) => f.name))
        } catch (e) {
            toast.error(apiErrorMessage(e))
            return
        }

        const initialItems: UploadItem[] = files.map((file, i) => ({
            key: fileKey(file),
            file,
            pictureId: slots[i].picture_id,
            presignedUrl: slots[i].presigned_url,
            status: 'pending',
            progress: 0,
        }))

        setItems(initialItems)
        setPhase('uploading')

        // Upload with max 4 concurrent workers
        const queue = [...initialItems]
        let firstSuccess = false

        async function worker() {
            while (true) {
                const item = queue.shift()
                if (!item) break

                const {pictureId, presignedUrl, file} = item

                patchItem(pictureId, {status: 'uploading'})
                try {
                    await uploadToS3(presignedUrl, file, (pct) =>
                        patchItem(pictureId, {progress: pct}),
                    )

                    patchItem(pictureId, {status: 'completing', progress: 97})

                    await completeUpload(pictureId, {
                        mime_type: file.type || undefined,
                        file_size: file.size,
                        initial_tags: tags.length ? tags : undefined,
                    })

                    patchItem(pictureId, {status: 'done', progress: 100})

                    if (!firstSuccess) {
                        firstSuccess = true
                        queryClient.invalidateQueries({queryKey: queryKeys.pictures()})
                    }
                } catch (e) {
                    patchItem(pictureId, {
                        status: 'error',
                        error: e instanceof Error ? e.message : 'Upload failed',
                    })
                }
            }
        }

        await Promise.all(Array.from({length: Math.min(4, initialItems.length)}, worker))

        setPhase('complete')
        queryClient.invalidateQueries({queryKey: queryKeys.pictures()})
        if (tags.length) queryClient.invalidateQueries({queryKey: queryKeys.tags()})
    }

    function resetForMore() {
        setFiles([])
        setItems([])
        setPhase('idle')
    }

    // Derived counts
    const doneCount = items.filter((i) => i.status === 'done').length
    const errorCount = items.filter((i) => i.status === 'error').length
    const activeCount = items.filter((i) => i.status === 'uploading' || i.status === 'completing').length
    const isUploading = phase === 'uploading'

    return (
        <Dialog
            open={open}
            onOpenChange={(o) => {
                if (!o && isUploading) return
                onOpenChange(o)
            }}
        >
            <DialogContent
                className="flex max-h-[90vh] max-w-2xl flex-col gap-0 overflow-hidden p-0"
                onInteractOutside={(e) => {
                    if (isUploading) e.preventDefault()
                }}
            >
                <DialogHeader className="px-6 pt-6 pb-4">
                    <DialogTitle>Upload photos</DialogTitle>
                </DialogHeader>

                <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-2">
                    {/* ── Dropzone / file list ─────────────────────────────── */}
                    <div
                        ref={dropzoneRef}
                        onDragOver={phase === 'idle' ? onDragOver : undefined}
                        onDragLeave={phase === 'idle' ? onDragLeave : undefined}
                        onDrop={phase === 'idle' ? onDrop : undefined}
                    >
                        {phase === 'idle' && files.length === 0 && (
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
                                    <p className="text-sm font-medium">Drop photos here</p>
                                    <p className="mt-0.5 text-xs text-muted-foreground">
                                        or click to browse your files
                                    </p>
                                </div>
                            </button>
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
                        {phase !== 'idle' && (
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
                                            <StatusIcon status={item.status}/>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        )}
                    </div>

                    {/* ── Add more / status summary ────────────────────────── */}
                    {phase === 'idle' && files.length > 0 && (
                        <div className="flex items-center gap-3">
                            <button
                                type="button"
                                className="text-xs text-primary hover:underline"
                                onClick={() => fileInputRef.current?.click()}
                            >
                                + Add more photos
                            </button>
                            <span className="text-xs text-muted-foreground">
                                {files.length} file{files.length !== 1 ? 's' : ''} selected
                            </span>
                        </div>
                    )}

                    {phase === 'uploading' && (
                        <p className="text-sm text-muted-foreground">
                            {activeCount > 0
                                ? `Uploading… ${doneCount} of ${items.length} done`
                                : 'Finishing up…'}
                        </p>
                    )}

                    {phase === 'complete' && (
                        <p className="text-sm">
                            {errorCount === 0 ? (
                                <span className="text-emerald-500">
                                    {doneCount} photo{doneCount !== 1 ? 's' : ''} uploaded successfully.
                                </span>
                            ) : (
                                <>
                                    {doneCount} uploaded
                                    {errorCount > 0 && (
                                        <span className="text-destructive">
                                            {' '}· {errorCount} failed
                                        </span>
                                    )}
                                </>
                            )}
                        </p>
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
                                disabled={isUploading}
                            >
                                Cancel
                            </Button>
                            <Button
                                onClick={startUpload}
                                disabled={files.length === 0 || isUploading}
                            >
                                {isUploading ? (
                                    <>
                                        <Loader2 className="mr-2 h-4 w-4 animate-spin"/>
                                        Uploading…
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
                        addFiles(e.target.files)
                        e.target.value = ''
                    }
                }}
            />
        </Dialog>
    )
}
