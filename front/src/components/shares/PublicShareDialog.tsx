import {useEffect, useState} from 'react'
import {CalendarClock, Link2} from 'lucide-react'
import {toast} from 'sonner'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {usePublicShareMutations} from '@/hooks/usePublicShares'
import type {PublicShareBody, PublicShareSummary} from '@/api/publicShares'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Textarea} from '@/components/ui/textarea'
import {Switch} from '@/components/ui/switch'
import {Label} from '@/components/ui/label'
import {Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger} from '@/components/ui/dialog'
import {TagPicker} from '@/components/tags/TagPicker'
import {DateTimePickerPopover, formatNaive} from '@/components/photos/detail/DateTimePickerPopover'

/**
 * Create or edit a public share link. Pass `share` to edit an existing one (its tag is immutable, so
 * the tag picker is read-only and the password field defaults to "keep current"). Otherwise it creates.
 */
export function PublicShareDialog({
                                      share,
                                      initialTag,
                                      open: openProp,
                                      onOpenChange,
                                      showTrigger = true,
                                      triggerLabel = 'New public share link',
                                      triggerVariant = 'secondary',
                                  }: {
    share?: PublicShareSummary
    initialTag?: string
    open?: boolean
    onOpenChange?: (v: boolean) => void
    showTrigger?: boolean
    triggerLabel?: string
    triggerVariant?: 'secondary' | 'ghost' | 'outline'
}) {
    const editing = !!share
    const [internalOpen, setInternalOpen] = useState(false)
    const open = openProp ?? internalOpen
    const setOpen = onOpenChange ?? setInternalOpen
    const {create, update} = usePublicShareMutations()

    const [tag, setTag] = useState(share?.tag_path ?? initialTag ?? '')
    const [name, setName] = useState(share?.name ?? '')
    const [message, setMessage] = useState(share?.message ?? '')
    const [password, setPassword] = useState('')
    // In edit mode the stored password is kept unless the user opts to change it.
    const [changePassword, setChangePassword] = useState(!editing)
    const [expires, setExpires] = useState<string | null>(share?.expires_at ?? null)
    const [allowOriginals, setAllowOriginals] = useState(share?.permissions.allow_originals ?? true)
    const [allowUpload, setAllowUpload] = useState(share?.permissions.allow_upload ?? false)
    const [allowShareBack, setAllowShareBack] = useState(share?.permissions.allow_share_back ?? false)
    const [convExif, setConvExif] = useState(share?.permissions.conv_allow_exif_edit ?? false)
    const [convFuture, setConvFuture] = useState(share?.permissions.conv_future ?? true)

    // Re-seed from props whenever the dialog opens (edit target or pre-filled tag can change).
    useEffect(() => {
        if (!open) return
        if (share) {
            setTag(share.tag_path)
            setName(share.name)
            setMessage(share.message ?? '')
            setExpires(share.expires_at ?? null)
            setAllowOriginals(share.permissions.allow_originals)
            setAllowUpload(share.permissions.allow_upload)
            setAllowShareBack(share.permissions.allow_share_back)
            setConvExif(share.permissions.conv_allow_exif_edit)
            setConvFuture(share.permissions.conv_future)
            setPassword('')
            setChangePassword(false)
        } else if (initialTag) {
            setTag(initialTag)
        }
    }, [open, share, initialTag])

    const reset = () => {
        setName('')
        setMessage('')
        setPassword('')
        setChangePassword(true)
        setExpires(null)
        setAllowOriginals(true)
        setAllowUpload(false)
        setAllowShareBack(false)
        setConvExif(false)
        setConvFuture(true)
    }

    const pending = editing ? update.isPending : create.isPending

    const submit = async () => {
        if (!tag || !name.trim()) {
            toast.error('A tag and a name are required.')
            return
        }
        const body: PublicShareBody = {
            tag_path: tag,
            name: name.trim(),
            message: message.trim() || null,
            // Create: always set. Edit: keep unless the user chose to change it (blank ⇒ removes it).
            password: changePassword ? password || null : null,
            keep_password: editing && !changePassword,
            expires_at: expires || null,
            allow_originals: allowOriginals,
            allow_upload: allowUpload,
            // Share-back needs originals (no convert ⇒ no share possible); forced on when uploads allowed.
            allow_share_back: allowOriginals && (allowShareBack || allowUpload),
            conv_allow_exif_edit: convExif,
            conv_future: convFuture,
        }
        try {
            if (editing) await update.mutateAsync({id: share!.id, body})
            else await create.mutateAsync(body)
            toast.success(editing ? 'Public share link updated.' : 'Public share link created.')
            if (!editing) reset()
            setOpen(false)
        } catch (e) {
            toast.error(apiErrorMessage(e))
        }
    }

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            {showTrigger && (
                <DialogTrigger asChild>
                    <Button size="sm" variant={triggerVariant}>
                        <Link2 className="mr-1.5 h-4 w-4"/> {triggerLabel}
                    </Button>
                </DialogTrigger>
            )}
            <DialogContent className="max-h-[90vh] max-w-lg overflow-y-auto">
                <DialogHeader>
                    <DialogTitle>{editing ? 'Edit public share link' : 'Create a public share link'}</DialogTitle>
                </DialogHeader>
                <div className="space-y-4">
                    {/* Tag — same layout as CreateShareDialog; no create-new (share an existing tag). */}
                    <div className="space-y-1.5">
                        <Label>Tag</Label>
                        {editing ? (
                            <span className="block rounded bg-muted px-2 py-1 text-sm">{TagPath.toDisplay(tag)}</span>
                        ) : (
                            <div className="flex min-w-0 items-center gap-2">
                                <TagPicker
                                    onSelect={setTag}
                                    triggerLabel={tag ? 'Change tag' : 'Choose tag'}
                                    allowCreate={false}
                                    allowProtected
                                />
                                {tag && (
                                    <span className="min-w-0 truncate text-sm text-muted-foreground">{TagPath.toDisplay(tag)}</span>
                                )}
                            </div>
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
                        label="Allow downloading full-size photos"
                        hint="Visitors can download originals, save copies, and transform this public share into a private tag share on their account."
                        checked={allowOriginals}
                        onChange={setAllowOriginals}
                    />
                    {!allowOriginals && (
                        <p className="rounded border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
                            Visitors will only see thumbnails. No full-size downloads or camera/location info (EXIF is removed).
                        </p>
                    )}
                    <Toggle
                        label="Let visitors upload photos"
                        hint="Anyone with the link can upload photos to your gallery (only videos and photos allowed, no duplicates allowed)."
                        checked={allowUpload}
                        onChange={setAllowUpload}
                    />
                    {/* Permissions of the private share a visitor creates by adding this album to their
                        account. Only relevant when originals are allowed (no convert ⇒ no share possible). */}
                    {allowOriginals && (
                        <div className="space-y-3 rounded-md border border-border bg-muted/20 p-3">
                            <p className="text-xs font-medium text-muted-foreground">
                                When someone convert this share to a private share:
                            </p>
                            <Toggle
                                label="Allow ShareBack"
                                hint={allowUpload ? 'Always on while uploads are allowed.' : undefined}
                                checked={allowShareBack || allowUpload}
                                disabled={allowUpload}
                                onChange={setAllowShareBack}
                            />
                            <Toggle label="Allow edit EXIF" checked={convExif} onChange={setConvExif}/>
                            <Toggle label="Share future additions" checked={convFuture} onChange={setConvFuture}/>
                        </div>
                    )}

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <div className="flex items-center justify-between gap-2">
                                <Label>Password (optional)</Label>
                                {editing && (
                                    <button
                                        type="button"
                                        className="text-xs text-primary hover:underline"
                                        onClick={() => setChangePassword((v) => !v)}
                                    >
                                        {changePassword ? 'Keep current' : 'Change'}
                                    </button>
                                )}
                            </div>
                            <Input
                                type="password"
                                value={password}
                                disabled={editing && !changePassword}
                                onChange={(e) => setPassword(e.target.value)}
                                placeholder={
                                    editing && !changePassword
                                        ? share?.has_password
                                            ? '•••••• (unchanged)'
                                            : 'No password'
                                        : 'No password'
                                }
                            />
                            {editing && changePassword && (
                                <p className="text-[11px] text-muted-foreground">Leave blank to remove the password.</p>
                            )}
                        </div>
                        <div className="space-y-1.5">
                            <Label>Expires (optional)</Label>
                            <DateTimePickerPopover value={expires} onChange={setExpires} disablePast>
                                <Button type="button" variant="outline" className="w-full justify-start gap-2 font-normal">
                                    <CalendarClock className="h-4 w-4 shrink-0 text-muted-foreground"/>
                                    <span className={expires ? '' : 'text-muted-foreground'}>
                                        {expires ? formatNaive(expires) : 'No expiry'}
                                    </span>
                                </Button>
                            </DateTimePickerPopover>
                        </div>
                    </div>

                    <div className="flex justify-end gap-2">
                        <Button variant="ghost" onClick={() => setOpen(false)}>
                            Cancel
                        </Button>
                        <Button onClick={submit} disabled={pending || !tag || !name.trim()}>
                            {editing ? 'Save changes' : 'Create link'}
                        </Button>
                    </div>
                </div>
            </DialogContent>
        </Dialog>
    )
}

function Toggle({
                    label,
                    hint,
                    checked,
                    onChange,
                    disabled,
                }: {
    label: string
    hint?: string
    checked: boolean
    onChange: (v: boolean) => void
    disabled?: boolean
}) {
    return (
        <label className="flex items-start justify-between gap-3 text-sm">
            <span className="flex min-w-0 flex-col">
                <span className={disabled ? 'text-muted-foreground' : ''}>{label}</span>
                {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
            </span>
            <Switch checked={checked} onCheckedChange={onChange} disabled={disabled} className="mt-0.5 shrink-0"/>
        </label>
    )
}
