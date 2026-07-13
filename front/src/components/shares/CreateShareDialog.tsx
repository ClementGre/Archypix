import {useEffect, useMemo, useState} from 'react'
import {AlertCircle, Check, Loader2, Plus, RotateCw, X} from 'lucide-react'
import {toast} from 'sonner'
import {useQueryClient} from '@tanstack/react-query'
import {Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle, DialogTrigger} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Textarea} from '@/components/ui/textarea'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {TagPicker} from '@/components/tags/TagPicker'
import {ContactInput} from '@/components/common/ContactInput'
import {createOutgoingShare} from '@/api/shares'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {formatIdentity, parseIdentity} from '@/lib/identity'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {useIncomingShares} from '@/hooks/useShares'
import {useAuthStore} from '@/stores/auth'
import type {IncomingShareResponse} from '@/lib/types'

const NAME_MAX = 64
const MESSAGE_MAX = 1000
const NONE = '__none__'

type RowStatus = 'pending' | 'creating' | 'done' | 'error'

interface Recipient {
    key: string
    /** `@username:domain` (an instance-less `@alice` defaults to the global domain on submit). */
    value: string
}

interface RowProgress {
    status: RowStatus
    error?: string
}

let recipientSeq = 0
const newRecipient = (): Recipient => ({key: `r${recipientSeq++}`, value: ''})
const recipientFor = (share: IncomingShareResponse): Recipient => ({
    key: `r${recipientSeq++}`,
    value: formatIdentity({username: share.sender_username, instance: share.sender_instance}),
})

/** Resolve a recipient field to concrete identity parts (default instance = global domain). */
const parseRecipient = (value: string) => parseIdentity(value, GLOBAL_DOMAIN)

export interface CreateShareDialogProps {
    /** Controlled open state. When omitted, the dialog manages its own state via the default trigger. */
    open?: boolean
    onOpenChange?: (open: boolean) => void
    /** Render the default "New share" trigger button (default true). */
    showTrigger?: boolean
    /** Open pre-configured as a ShareBack of this incoming share. */
    initialShareback?: IncomingShareResponse | null
    /** Pre-fill the tag (wire form) — e.g. the local tag the ShareBack source is mapped to. Stays editable. */
    initialTag?: string
}

export function CreateShareDialog({
                                      open: controlledOpen,
                                      onOpenChange,
                                      showTrigger = true,
                                      initialShareback,
                                      initialTag,
                                  }: CreateShareDialogProps = {}) {
    const [uncontrolledOpen, setUncontrolledOpen] = useState(false)
    const open = controlledOpen ?? uncontrolledOpen
    const setOpen = (o: boolean) => {
        onOpenChange?.(o)
        if (controlledOpen === undefined) setUncontrolledOpen(o)
    }

    const [name, setName] = useState('')
    const [message, setMessage] = useState('')
    const [tag, setTag] = useState('')
    const [allowShareBack, setAllowShareBack] = useState(true)
    const [allowExifEdit, setAllowExifEdit] = useState(false)
    const [future, setFuture] = useState(true)
    const [sharebackOfId, setSharebackOfId] = useState('') // selected incoming share id, '' = none
    const [recipients, setRecipients] = useState<Recipient[]>(() => [newRecipient()])
    const [submitting, setSubmitting] = useState(false)
    const [progress, setProgress] = useState<Record<string, RowProgress>>({})

    const queryClient = useQueryClient()
    const {data: incomingShares} = useIncomingShares()
    const currentUser = useAuthStore((s) => s.user)
    const currentInstance = useAuthStore((s) => s.instance)

    // Frontend-only guard: you can't share with yourself (the backend would reject it too, but this
    // avoids a pointless round-trip and explains why).
    const isSelf = (value: string): boolean => {
        const id = parseRecipient(value)
        if (!id || !currentUser || !currentInstance) return false
        return (
            id.username.toLowerCase() === currentUser.username.toLowerCase() &&
            id.instance.toLowerCase() === currentInstance.toLowerCase()
        )
    }

    // Incoming shares eligible as a ShareBack target (still live).
    const sharebackOptions = useMemo(
        () => (incomingShares ?? []).filter((s) => s.status === 'active' || s.status === 'pending'),
        [incomingShares],
    )
    const selectedIncoming = useMemo(
        () => (incomingShares ?? []).find((s) => s.id === sharebackOfId) ?? null,
        [incomingShares, sharebackOfId],
    )
    const isShareBack = !!selectedIncoming

    // (Re)initialise on open; reset everything on close. When opened as a ShareBack, lock the
    // recipient to the original sender.
    useEffect(() => {
        if (open) {
            const seed = initialShareback ?? null
            setSharebackOfId(seed?.id ?? '')
            setRecipients(seed ? [recipientFor(seed)] : [newRecipient()])
            setTag(initialTag ?? '')
            // Reuse the original share's name so the owner recognises the ShareBack (still editable).
            setName(seed?.name ?? '')
        } else {
            setName('')
            setMessage('')
            setTag('')
            setAllowShareBack(true)
            setAllowExifEdit(false)
            setFuture(true)
            setSharebackOfId('')
            setRecipients([newRecipient()])
            setSubmitting(false)
            setProgress({})
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open])

    const onSelectShareback = (value: string) => {
        if (value === NONE) {
            setSharebackOfId('')
            setRecipients([newRecipient()])
            return
        }
        const share = (incomingShares ?? []).find((s) => s.id === value)
        if (!share) return
        setSharebackOfId(value)
        setRecipients([recipientFor(share)])
    }

    // Editing a recipient clears any prior error/status for that row (it becomes retriable-fresh).
    const patchRecipient = (key: string, value: string) => {
        setRecipients((prev) => prev.map((r) => (r.key === key ? {...r, value} : r)))
        setProgress((prev) => {
            if (!prev[key]) return prev
            const next = {...prev}
            delete next[key]
            return next
        })
    }

    const validRecipients = recipients.filter((r) => parseRecipient(r.value) !== null && !isSelf(r.value))
    const trimmedName = name.trim()
    const pendingRecipients = validRecipients.filter((r) => progress[r.key]?.status !== 'done')
    const allDone = validRecipients.length > 0 && pendingRecipients.length === 0
    const anyError = Object.values(progress).some((p) => p.status === 'error')
    const canSubmit =
        !submitting &&
        trimmedName.length > 0 &&
        trimmedName.length <= NAME_MAX &&
        !!tag &&
        validRecipients.length > 0 &&
        pendingRecipients.length > 0

    async function runShares(targets: Recipient[]) {
        setSubmitting(true)
        setProgress((prev) => ({
            ...prev,
            ...Object.fromEntries(targets.map((r) => [r.key, {status: 'pending' as RowStatus}])),
        }))

        // A ShareBack references the original outgoing share (the incoming share's outgoing_share_id).
        const sharebackOf = selectedIncoming?.outgoing_share_id

        let succeeded = 0
        let failed = 0
        for (const r of targets) {
            const id = parseRecipient(r.value)
            if (!id) continue
            setProgress((prev) => ({...prev, [r.key]: {status: 'creating'}}))
            try {
                await createOutgoingShare({
                    tag_path: tag,
                    name: trimmedName,
                    message: message.trim() || undefined,
                    recipient_username: id.username,
                    recipient_instance: id.instance,
                    allow_share_back: allowShareBack,
                    allow_exif_edit: allowExifEdit,
                    future,
                    shareback_of: sharebackOf,
                })
                succeeded++
                setProgress((prev) => ({...prev, [r.key]: {status: 'done'}}))
            } catch (err) {
                failed++
                setProgress((prev) => ({...prev, [r.key]: {status: 'error', error: apiErrorMessage(err)}}))
            }
        }

        if (succeeded > 0) void queryClient.invalidateQueries({queryKey: ['shares']})
        setSubmitting(false)

        if (failed === 0 && succeeded > 0) {
            toast.success(`Share created for ${succeeded} recipient${succeeded !== 1 ? 's' : ''}`)
        } else if (failed > 0) {
            toast.error(`${failed} share${failed !== 1 ? 's' : ''} failed — edit and retry`)
        }
    }

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault()
        if (!canSubmit) return
        void runShares(pendingRecipients)
    }

    return (
        <Dialog
            open={open}
            onOpenChange={(o) => {
                if (!o && submitting) return
                setOpen(o)
            }}
        >
            {showTrigger && (
                <DialogTrigger asChild>
                    <Button size="sm" className="gap-1.5">
                        <Plus className="h-3.5 w-3.5"/>
                        New share
                    </Button>
                </DialogTrigger>
            )}
            <DialogContent
                className="max-w-md"
                onInteractOutside={(e) => {
                    if (submitting) e.preventDefault()
                }}
            >
                <DialogHeader>
                    <DialogTitle>{isShareBack ? 'Create ShareBack' : 'Create outgoing share'}</DialogTitle>
                </DialogHeader>
                <form onSubmit={handleSubmit} className="min-w-0 space-y-4">
                    {/* Share back of */}
                    <div className="space-y-1.5">
                        <Label>Share back of <span className="text-muted-foreground">(optional)</span></Label>
                        <Select
                            value={sharebackOfId || NONE}
                            onValueChange={onSelectShareback}
                            disabled={submitting}
                        >
                            <SelectTrigger>
                                <SelectValue placeholder="Not a ShareBack"/>
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={NONE}>Not a ShareBack</SelectItem>
                                {sharebackOptions.map((s) => (
                                    <SelectItem key={s.id} value={s.id}>
                                        {s.name} — @{s.sender_username}:{s.sender_instance}
                                    </SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                        {isShareBack && (
                            <p className="text-[11px] text-muted-foreground">
                                Shared back to @{selectedIncoming!.sender_username}:{selectedIncoming!.sender_instance}.
                                {selectedIncoming!.allow_share_back
                                    ? ' Auto-accepted (the sender allows ShareBack).'
                                    : ' The sender must accept it manually.'}
                            </p>
                        )}
                    </div>

                    {/* Name */}
                    <div className="space-y-1.5">
                        <div className="flex items-center justify-between">
                            <Label htmlFor="share-name">Name</Label>
                            <span className="text-[11px] text-muted-foreground">
                                {trimmedName.length}/{NAME_MAX}
                            </span>
                        </div>
                        <Input
                            id="share-name"
                            value={name}
                            maxLength={NAME_MAX}
                            onChange={(e) => setName(e.target.value)}
                            placeholder="e.g. Alps 2024"
                            disabled={submitting}
                        />
                    </div>

                    {/* Message */}
                    <div className="space-y-1.5">
                        <div className="flex items-center justify-between">
                            <Label htmlFor="share-message">Message <span className="text-muted-foreground">(optional)</span></Label>
                            <span className="text-[11px] text-muted-foreground">
                                {message.length}/{MESSAGE_MAX}
                            </span>
                        </div>
                        <Textarea
                            id="share-message"
                            value={message}
                            maxLength={MESSAGE_MAX}
                            onChange={(e) => setMessage(e.target.value)}
                            placeholder="A note shown to the recipient"
                            className="min-h-[60px]"
                            disabled={submitting}
                        />
                    </div>

                    {/* Tag */}
                    <div className="space-y-1.5">
                        <Label>Tag</Label>
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
                    </div>

                    {/* Recipients */}
                    <div className="space-y-1.5">
                        <Label>{isShareBack ? 'Recipient' : 'Recipients'}</Label>
                        <div className="space-y-2">
                            {recipients.map((r) => {
                                const status = progress[r.key]?.status
                                const locked = submitting || status === 'done' || status === 'creating' || isShareBack
                                return (
                                    <div key={r.key} className="space-y-1">
                                        <div className="flex items-start gap-1.5">
                                            <ContactInput
                                                value={r.value}
                                                onChange={(v) => patchRecipient(r.key, v)}
                                                allowCustomValues={false}
                                                defaultInstance={GLOBAL_DOMAIN}
                                                disabled={locked}
                                                className="flex-1"
                                            />
                                            {status === 'done' ? (
                                                <Check className="mt-2 h-4 w-4 shrink-0 text-emerald-500"/>
                                            ) : status === 'creating' ? (
                                                <Loader2 className="mt-2 h-4 w-4 shrink-0 animate-spin text-primary"/>
                                            ) : status === 'error' ? (
                                                <Button
                                                    type="button"
                                                    size="icon"
                                                    variant="ghost"
                                                    className="h-8 w-8 shrink-0 text-muted-foreground hover:text-primary"
                                                    title="Retry"
                                                    disabled={submitting || parseRecipient(r.value) === null}
                                                    onClick={() => void runShares([r])}
                                                >
                                                    <RotateCw className="h-3.5 w-3.5"/>
                                                </Button>
                                            ) : (
                                                !isShareBack && (
                                                    <Button
                                                        type="button"
                                                        size="icon"
                                                        variant="ghost"
                                                        className="h-8 w-8 shrink-0 text-muted-foreground hover:text-destructive disabled:opacity-30"
                                                        title="Remove recipient"
                                                        disabled={submitting || recipients.length === 1}
                                                        onClick={() => setRecipients((prev) => prev.filter((x) => x.key !== r.key))}
                                                    >
                                                        <X className="h-3.5 w-3.5"/>
                                                    </Button>
                                                )
                                            )}
                                        </div>
                                        {status === 'error' && (
                                            <p className="flex items-start gap-1 break-words pl-1 text-[11px] text-destructive">
                                                <AlertCircle className="mt-0.5 h-3 w-3 shrink-0"/>
                                                <span>{progress[r.key]?.error}</span>
                                            </p>
                                        )}
                                        {isSelf(r.value) && (
                                            <p className="flex items-start gap-1 pl-1 text-[11px] text-destructive">
                                                <AlertCircle className="mt-0.5 h-3 w-3 shrink-0"/>
                                                <span>You can't share with yourself.</span>
                                            </p>
                                        )}
                                    </div>
                                )
                            })}
                        </div>
                        {!isShareBack && (
                            <button
                                type="button"
                                className="text-xs text-primary hover:underline disabled:opacity-50"
                                disabled={submitting}
                                onClick={() => setRecipients((prev) => [...prev, newRecipient()])}
                            >
                                + Add recipient
                            </button>
                        )}
                    </div>

                    {/* Toggles */}
                    <div className="flex items-center justify-between">
                        <Label htmlFor="allow-share-back">Allow ShareBack</Label>
                        <Switch
                            id="allow-share-back"
                            checked={allowShareBack}
                            onCheckedChange={setAllowShareBack}
                            disabled={submitting}
                        />
                    </div>
                    <div className="flex items-center justify-between">
                        <Label htmlFor="allow-exif-edit">Allow recipients to edit EXIF</Label>
                        <Switch
                            id="allow-exif-edit"
                            checked={allowExifEdit}
                            onCheckedChange={setAllowExifEdit}
                            disabled={submitting}
                        />
                    </div>
                    <div className="flex items-center justify-between">
                        <Label htmlFor="future">Share future additions</Label>
                        <Switch
                            id="future"
                            checked={future}
                            onCheckedChange={setFuture}
                            disabled={submitting}
                        />
                    </div>

                    <DialogFooter className="gap-2 sm:gap-2">
                        {allDone ? (
                            <Button type="button" className="w-full" onClick={() => setOpen(false)}>
                                Done
                            </Button>
                        ) : (
                            <Button type="submit" className="w-full" disabled={!canSubmit}>
                                {submitting && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                                {submitting
                                    ? 'Creating…'
                                    : anyError
                                        ? 'Retry failed shares'
                                        : pendingRecipients.length > 1
                                            ? `Create ${pendingRecipients.length} shares`
                                            : isShareBack
                                                ? 'Create ShareBack'
                                                : 'Create share'}
                            </Button>
                        )}
                    </DialogFooter>
                </form>
            </DialogContent>
        </Dialog>
    )
}
