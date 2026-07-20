import {useState} from 'react'
import {Check, Loader2, Upload, X} from 'lucide-react'
import {toast} from 'sonner'
import {useQueryClient} from '@tanstack/react-query'
import {publicCompleteUpload, publicUploadBatch} from '@/api/publicShares'
import {apiErrorMessage} from '@/api/client'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Dialog, DialogContent, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {usePublicShare} from '@/components/public/context'

type SlotState = 'pending' | 'uploading' | 'done' | 'rejected' | 'error'

/** SHA-256 lowercase hex of a file — the same digest the worker computes (for upload-time dedup). */
async function sha256Hex(file: File): Promise<string> {
    const buf = await file.arrayBuffer()
    const digest = await crypto.subtle.digest('SHA-256', buf)
    return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

export function PublicUploadDialog({open, onOpenChange}: { open: boolean; onOpenChange: (v: boolean) => void }) {
    const {backendUrl, token} = usePublicShare()
    const qc = useQueryClient()
    const [name, setName] = useState('')
    const [files, setFiles] = useState<File[]>([])
    const [states, setStates] = useState<SlotState[]>([])
    const [busy, setBusy] = useState(false)

    const reset = () => {
        setFiles([])
        setStates([])
        setBusy(false)
    }

    const submit = async () => {
        if (files.length === 0) return
        setBusy(true)
        setStates(files.map(() => 'pending'))
        try {
            const hashes = await Promise.all(files.map(sha256Hex))
            const slots = await publicUploadBatch(
                backendUrl,
                token,
                name,
                files.map((f, i) => ({filename: f.name, file_hash: hashes[i], size: f.size})),
            )
            const next: SlotState[] = [...states]
            for (let i = 0; i < slots.length; i++) {
                const slot = slots[i]
                if (slot.rejected || !slot.presigned_url) {
                    next[i] = 'rejected'
                    setStates([...next])
                    continue
                }
                next[i] = 'uploading'
                setStates([...next])
                try {
                    await fetch(slot.presigned_url, {method: 'PUT', body: files[i]})
                    await publicCompleteUpload(backendUrl, token, slot.picture_id, {
                        contributor_name: name,
                        mime_type: files[i].type || 'application/octet-stream',
                        file_size: files[i].size,
                        file_hash: hashes[i],
                    })
                    next[i] = 'done'
                } catch {
                    next[i] = 'error'
                }
                setStates([...next])
            }
            const done = next.filter((s) => s === 'done').length
            if (done > 0) {
                toast.success(`Contributed ${done} photo${done === 1 ? '' : 's'}.`)
                qc.invalidateQueries({queryKey: ['publicPictures', backendUrl, token]})
            }
        } catch (e) {
            toast.error(apiErrorMessage(e))
        } finally {
            setBusy(false)
        }
    }

    return (
        <Dialog
            open={open}
            onOpenChange={(v) => {
                if (!busy) {
                    if (!v) reset()
                    onOpenChange(v)
                }
            }}
        >
            <DialogContent className="max-w-lg">
                <DialogHeader>
                    <DialogTitle>Contribute to this album</DialogTitle>
                </DialogHeader>
                <div className="space-y-3">
                    <Input placeholder="Your name (shown as the credit)" value={name} onChange={(e) => setName(e.target.value)}/>
                    <label
                        className="flex cursor-pointer items-center justify-center gap-2 rounded-md border border-dashed border-border p-4 text-sm text-muted-foreground hover:bg-muted">
                        <Upload className="h-4 w-4"/>
                        {files.length ? `${files.length} file(s) selected` : 'Choose images or videos'}
                        <input
                            type="file"
                            multiple
                            accept="image/*,video/*"
                            className="hidden"
                            onChange={(e) => {
                                setFiles([...(e.target.files ?? [])])
                                setStates([])
                            }}
                        />
                    </label>

                    {files.length > 0 && (
                        <ul className="max-h-48 space-y-1 overflow-y-auto text-sm">
                            {files.map((f, i) => (
                                <li key={i} className="flex items-center justify-between gap-2 rounded px-2 py-1">
                                    <span className="min-w-0 truncate">{f.name}</span>
                                    <SlotIcon state={states[i]}/>
                                </li>
                            ))}
                        </ul>
                    )}

                    <div className="flex justify-end gap-2">
                        <Button variant="ghost" disabled={busy} onClick={() => onOpenChange(false)}>
                            Close
                        </Button>
                        <Button disabled={busy || files.length === 0 || !name.trim()} onClick={submit}>
                            {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                            Upload
                        </Button>
                    </div>
                    <p className="text-xs text-muted-foreground">
                        Photos you contribute become part of the owner's library. Duplicates of existing photos are skipped.
                    </p>
                </div>
            </DialogContent>
        </Dialog>
    )
}

function SlotIcon({state}: { state: SlotState | undefined }) {
    if (state === 'done') return <Check className="h-4 w-4 text-emerald-500"/>
    if (state === 'rejected') return <span className="text-xs text-amber-500">already in album</span>
    if (state === 'error') return <X className="h-4 w-4 text-destructive"/>
    if (state === 'uploading' || state === 'pending') return <Loader2 className="h-4 w-4 animate-spin text-muted-foreground"/>
    return null
}
