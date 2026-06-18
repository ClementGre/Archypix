import {useEffect, useMemo, useRef, useState} from 'react'
import {AlertCircle, Check, Loader2, Plus, X} from 'lucide-react'
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
import {createOutgoingShare} from '@/api/shares'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {useIncomingShares} from '@/hooks/useShares'
import type {IncomingShareResponse} from '@/lib/types'

const NAME_MAX = 64
const MESSAGE_MAX = 1000
const NONE = '__none__'

type RowStatus = 'pending' | 'creating' | 'done' | 'error'

interface Recipient {
    key: string
    username: string
    instance: string
}

interface RowProgress {
    status: RowStatus
    error?: string
}

let recipientSeq = 0
const newRecipient = (): Recipient => ({key: `r${recipientSeq++}`, username: '', instance: GLOBAL_DOMAIN})

/** Grouped `@username:instance` field styled as a single input. Typing `:` in
 *  the username sub-field advances focus to the instance sub-field. */
function RecipientField({
                            recipient,
                            disabled,
                            onChange,
                        }: {
    recipient: Recipient
    disabled: boolean
    onChange: (patch: Partial<Recipient>) => void
}) {
    const instanceRef = useRef<HTMLInputElement>(null)
    return (
        <div
            className="flex min-w-0 flex-1 items-center rounded-md border border-input bg-background px-2 text-sm focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 focus-within:ring-offset-background">
            <span className="select-none text-muted-foreground">@</span>
            <input
                value={recipient.username}
                disabled={disabled}
                onChange={(e) => onChange({username: e.target.value})}
                onKeyDown={(e) => {
                    if (e.key === ':') {
                        e.preventDefault()
                        instanceRef.current?.focus()
                    }
                }}
                placeholder="username"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                className="min-w-0 flex-1 bg-transparent px-1 py-1.5 outline-none disabled:opacity-50"
            />
            <span className="select-none text-muted-foreground">:</span>
            <input
                ref={instanceRef}
                value={recipient.instance}
                disabled={disabled}
                onChange={(e) => onChange({instance: e.target.value})}
                placeholder="instance"
                autoCapitalize="none"
                autoCorrect="off"
                spellCheck={false}
                className="min-w-0 flex-1 bg-transparent px-1 py-1.5 outline-none disabled:opacity-50"
            />
        </div>
    )
}

function StatusIcon({status}: { status: RowStatus }) {
    if (status === 'done') return <Check className="h-4 w-4 shrink-0 text-emerald-500"/>
    if (status === 'error') return <AlertCircle className="h-4 w-4 shrink-0 text-destructive"/>
    if (status === 'creating') return <Loader2 className="h-4 w-4 shrink-0 animate-spin text-primary"/>
    return <div className="h-4 w-4 shrink-0"/>
}

const recipientFor = (share: IncomingShareResponse): Recipient => ({
    key: `r${recipientSeq++}`,
    username: share.sender_username,
    instance: share.sender_instance,
})

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
                                      initialTag
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
    const [future, setFuture] = useState(true)
    const [sharebackOfId, setSharebackOfId] = useState('') // selected incoming share id, '' = none
    const [recipients, setRecipients] = useState<Recipient[]>(() => [newRecipient()])
    const [submitting, setSubmitting] = useState(false)
    const [progress, setProgress] = useState<Record<string, RowProgress>>({})

    const queryClient = useQueryClient()
    const {data: incomingShares} = useIncomingShares()

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

    const patchRecipient = (key: string, patch: Partial<Recipient>) =>
        setRecipients((prev) => prev.map((r) => (r.key === key ? {...r, ...patch} : r)))

    const validRecipients = recipients.filter((r) => r.username.trim() && r.instance.trim())
    const trimmedName = name.trim()
    const complete = Object.keys(progress).length > 0 && !submitting
    const canSubmit =
        !submitting && !complete && trimmedName.length > 0 && trimmedName.length <= NAME_MAX && !!tag && validRecipients.length > 0

    async function handleSubmit(e: React.FormEvent) {
        e.preventDefault()
        if (!canSubmit) return

        setSubmitting(true)
        setProgress(Object.fromEntries(validRecipients.map((r) => [r.key, {status: 'pending' as RowStatus}])))

        // A ShareBack references the original outgoing share (the incoming share's outgoing_share_id).
        const sharebackOf = selectedIncoming?.outgoing_share_id

        let succeeded = 0
        let failed = 0
        for (const r of validRecipients) {
            setProgress((prev) => ({...prev, [r.key]: {status: 'creating'}}))
            try {
                await createOutgoingShare({
                    tag_path: tag,
                    name: trimmedName,
                    message: message.trim() || undefined,
                    recipient_username: r.username.trim(),
                    recipient_instance: r.instance.trim(),
                    allow_share_back: allowShareBack,
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

        if (failed === 0) {
            toast.success(`Share created for ${succeeded} recipient${succeeded !== 1 ? 's' : ''}`)
        } else {
            toast.error(`${failed} of ${validRecipients.length} share${validRecipients.length !== 1 ? 's' : ''} failed`)
        }
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
                            disabled={submitting || complete}
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
                            disabled={submitting || complete}
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
                            disabled={submitting || complete}
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
                        <div className="space-y-1.5">
                            {recipients.map((r) => {
                                const rowProgress = progress[r.key]
                                return (
                                    <div key={r.key} className="flex items-center gap-1.5">
                                        <RecipientField
                                            recipient={r}
                                            disabled={submitting || complete || isShareBack}
                                            onChange={(patch) => patchRecipient(r.key, patch)}
                                        />
                                        {rowProgress ? (
                                            <StatusIcon status={rowProgress.status}/>
                                        ) : (
                                            !isShareBack && (
                                                <Button
                                                    type="button"
                                                    size="icon"
                                                    variant="ghost"
                                                    className="h-7 w-7 shrink-0 text-muted-foreground hover:text-destructive disabled:opacity-30"
                                                    title="Remove recipient"
                                                    disabled={recipients.length === 1}
                                                    onClick={() => setRecipients((prev) => prev.filter((x) => x.key !== r.key))}
                                                >
                                                    <X className="h-3.5 w-3.5"/>
                                                </Button>
                                            )
                                        )}
                                    </div>
                                )
                            })}
                        </div>
                        {/* Per-recipient error details */}
                        {Object.entries(progress)
                            .filter(([, p]) => p.status === 'error')
                            .map(([key, p]) => {
                                const r = recipients.find((x) => x.key === key)
                                return (
                                    <p key={key} className="break-words text-[11px] text-destructive">
                                        @{r?.username}:{r?.instance} — {p.error}
                                    </p>
                                )
                            })}
                        {!complete && !isShareBack && (
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
                            disabled={submitting || complete}
                        />
                    </div>
                    <div className="flex items-center justify-between">
                        <Label htmlFor="future">Share future additions</Label>
                        <Switch
                            id="future"
                            checked={future}
                            onCheckedChange={setFuture}
                            disabled={submitting || complete}
                        />
                    </div>

                    <DialogFooter className="gap-2 sm:gap-2">
                        {complete ? (
                            <Button type="button" className="w-full" onClick={() => setOpen(false)}>
                                Done
                            </Button>
                        ) : (
                            <Button type="submit" className="w-full" disabled={!canSubmit}>
                                {submitting && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                                {submitting
                                    ? 'Creating…'
                                    : validRecipients.length > 1
                                        ? `Create ${validRecipients.length} shares`
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
