import {useEffect, useState} from 'react'
import {Link2} from 'lucide-react'
import {toast} from 'sonner'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {usePublicShareMutations} from '@/hooks/usePublicShares'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Textarea} from '@/components/ui/textarea'
import {Switch} from '@/components/ui/switch'
import {Label} from '@/components/ui/label'
import {Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger} from '@/components/ui/dialog'
import {TagPicker} from '@/components/tags/TagPicker'

export function CreatePublicShareDialog({
                                            initialTag,
                                            open: openProp,
                                            onOpenChange,
                                            showTrigger = true,
                                        }: {
    initialTag?: string
    open?: boolean
    onOpenChange?: (v: boolean) => void
    showTrigger?: boolean
}) {
    const [internalOpen, setInternalOpen] = useState(false)
    const open = openProp ?? internalOpen
    const setOpen = onOpenChange ?? setInternalOpen
    const {create} = usePublicShareMutations()

    const [tag, setTag] = useState(initialTag ?? '')
    const [name, setName] = useState('')
    const [message, setMessage] = useState('')
    const [password, setPassword] = useState('')
    const [expires, setExpires] = useState('')
    const [allowOriginals, setAllowOriginals] = useState(true)
    const [allowUpload, setAllowUpload] = useState(false)
    const [allowShareBack, setAllowShareBack] = useState(false)
    const [convExif, setConvExif] = useState(false)
    const [convFuture, setConvFuture] = useState(true)

    useEffect(() => {
        if (open && initialTag) setTag(initialTag)
    }, [open, initialTag])

    const reset = () => {
        setName('')
        setMessage('')
        setPassword('')
        setExpires('')
        setAllowOriginals(true)
        setAllowUpload(false)
        setAllowShareBack(false)
        setConvExif(false)
        setConvFuture(true)
    }

    const submit = async () => {
        if (!tag || !name.trim()) {
            toast.error('A tag and a name are required.')
            return
        }
        try {
            await create.mutateAsync({
                tag_path: tag,
                name: name.trim(),
                message: message.trim() || null,
                password: password || null,
                expires_at: expires ? `${expires}T23:59:59` : null,
                allow_originals: allowOriginals,
                allow_upload: allowUpload,
                // ShareBack is forced on when uploads are allowed (backend enforces this too).
                allow_share_back: allowShareBack || allowUpload,
                conv_allow_exif_edit: convExif,
                conv_future: convFuture,
            })
            toast.success('Public link created.')
            reset()
            setOpen(false)
        } catch (e) {
            toast.error(apiErrorMessage(e))
        }
    }

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            {showTrigger && (
                <DialogTrigger asChild>
                    <Button size="sm" variant="secondary">
                        <Link2 className="mr-1.5 h-4 w-4"/> New public link
                    </Button>
                </DialogTrigger>
            )}
            <DialogContent className="max-h-[90vh] max-w-lg overflow-y-auto">
                <DialogHeader>
                    <DialogTitle>Create a public link</DialogTitle>
                </DialogHeader>
                <div className="space-y-4">
                    <div className="space-y-1.5">
                        <Label>Tag to share</Label>
                        {tag ? (
                            <div className="flex items-center gap-2">
                                <span className="rounded bg-muted px-2 py-1 text-sm">{TagPath.toDisplay(tag)}</span>
                                <TagPicker
                                    allowProtected
                                    onSelect={setTag}
                                    trigger={<Button size="sm" variant="ghost">Change</Button>}
                                />
                            </div>
                        ) : (
                            <TagPicker allowProtected onSelect={setTag} triggerLabel="Pick a tag"/>
                        )}
                    </div>

                    <div className="space-y-1.5">
                        <Label>Name</Label>
                        <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Alps 2024" maxLength={64}/>
                    </div>
                    <div className="space-y-1.5">
                        <Label>Message (optional)</Label>
                        <Textarea value={message} onChange={(e) => setMessage(e.target.value)} rows={2}/>
                    </div>

                    <Toggle
                        label="Allow originals (download, save a copy, convert)"
                        checked={allowOriginals}
                        onChange={setAllowOriginals}
                    />
                    {!allowOriginals && (
                        <p className="rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
                            Viewers won't be able to see or download original files — only the thumbnails shown, with EXIF/GPS
                            removed.
                        </p>
                    )}
                    <Toggle label="Allow anonymous contributions" checked={allowUpload} onChange={setAllowUpload}/>
                    <Toggle
                        label="Allow authenticated share-back"
                        checked={allowShareBack || allowUpload}
                        disabled={allowUpload}
                        onChange={setAllowShareBack}
                    />
                    {allowOriginals && (
                        <>
                            <Toggle
                                label="Subscribers may edit EXIF (derived share)"
                                checked={convExif}
                                onChange={setConvExif}
                            />
                            <Toggle label="Subscribers get future additions" checked={convFuture} onChange={setConvFuture}/>
                        </>
                    )}

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <Label>Password (optional)</Label>
                            <Input
                                type="password"
                                value={password}
                                onChange={(e) => setPassword(e.target.value)}
                                placeholder="No password"
                            />
                        </div>
                        <div className="space-y-1.5">
                            <Label>Expires (optional)</Label>
                            <Input type="date" value={expires} onChange={(e) => setExpires(e.target.value)}/>
                        </div>
                    </div>

                    <div className="flex justify-end gap-2">
                        <Button variant="ghost" onClick={() => setOpen(false)}>
                            Cancel
                        </Button>
                        <Button onClick={submit} disabled={create.isPending || !tag || !name.trim()}>
                            Create link
                        </Button>
                    </div>
                </div>
            </DialogContent>
        </Dialog>
    )
}

function Toggle({
                    label,
                    checked,
                    onChange,
                    disabled,
                }: {
    label: string
    checked: boolean
    onChange: (v: boolean) => void
    disabled?: boolean
}) {
    return (
        <label className="flex items-center justify-between gap-3 text-sm">
            <span className={disabled ? 'text-muted-foreground' : ''}>{label}</span>
            <Switch checked={checked} onCheckedChange={onChange} disabled={disabled}/>
        </label>
    )
}
