import {useState} from 'react'
import {Check, Copy, Link2, Loader2, Plus, Ticket, Trash2, UserRound} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Label} from '@/components/ui/label'
import {Badge} from '@/components/ui/badge'
import {Skeleton} from '@/components/ui/skeleton'
import {Checkbox} from '@/components/ui/checkbox'
import {NumberInput} from '@/components/ui/number-input'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import type {Invite, RegistrationMode} from '@/lib/types'

/** The frontend URL an invitee opens (this app's own origin, where the register page lives). */
export function inviteLink(code: string): string {
    return `${window.location.origin}/register?invite=${encodeURIComponent(code)}`
}

/** Display a 9-char code grouped for readability: `abcdefghi` → `ABC-DEF-GHI`. */
export function formatInviteCode(code: string): string {
    const up = code.toUpperCase()
    return up.length === 9 ? `${up.slice(0, 3)}-${up.slice(3, 6)}-${up.slice(6)}` : up
}

/** `max_uses === null` = tracking referral link (open-only); `0` = unlimited; `n` = capped. */
function isTracking(inv: Invite): boolean {
    return inv.max_uses === null
}

function CopyButton({text, label}: { text: string; label: string }) {
    const [copied, setCopied] = useState(false)
    const copy = async () => {
        try {
            await navigator.clipboard.writeText(text)
            setCopied(true)
            setTimeout(() => setCopied(false), 1500)
        } catch {
            toast.error('Could not copy')
        }
    }
    return (
        <Button variant="ghost" size="icon" className="h-7 w-7" onClick={copy} title={label} aria-label={label}>
            {copied ? <Check className="h-3.5 w-3.5 text-emerald-500"/> : <Copy className="h-3.5 w-3.5"/>}
        </Button>
    )
}

function statusOf(inv: Invite, mode: RegistrationMode | undefined): { label: string; inactive: boolean } {
    const expired = inv.expires_at ? new Date(inv.expires_at).getTime() < Date.now() : false
    const exhausted = inv.max_uses !== null && inv.max_uses > 0 && inv.uses >= inv.max_uses
    // A tracking referral is inactive once registration is no longer open (tombstoned).
    const strandedTracking = isTracking(inv) && mode !== undefined && mode !== 'open'
    if (expired) return {label: 'expired', inactive: true}
    if (exhausted) return {label: 'used up', inactive: true}
    if (strandedTracking) return {label: 'inactive (referrals need open registration)', inactive: true}
    if (isTracking(inv)) return {label: `${inv.uses} joined`, inactive: false}
    if (inv.max_uses === 0) return {label: `${inv.uses} used · unlimited`, inactive: false}
    return {label: `${inv.uses} / ${inv.max_uses} used`, inactive: false}
}

/** A tracking referral that no longer works because the instance switched to invite-only registration. */
function isStrandedReferral(inv: Invite, mode: RegistrationMode | undefined): boolean {
    return isTracking(inv) && mode !== undefined && mode !== 'open'
}

function InviteRow({inv, mode, onRevoke, showPin}: {
    inv: Invite
    mode: RegistrationMode | undefined
    onRevoke: (code: string) => Promise<void>
    showPin?: boolean
}) {
    // A stranded referral is shown greyed at the top, explained, with no copy/revoke actions.
    if (isStrandedReferral(inv, mode)) {
        return (
            <div className="flex items-start gap-2 border-b border-border/60 py-2 opacity-70 last:border-0">
                <Link2 className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"/>
                <div className="min-w-0 space-y-0.5">
                    <code
                        className="rounded bg-muted px-1.5 py-0.5 text-xs tracking-wider text-muted-foreground line-through">{formatInviteCode(inv.code)}</code>
                    <p className="text-xs text-muted-foreground">
                        This referral link no longer works — registration is now invite-only. Create invite codes below instead.
                    </p>
                </div>
            </div>
        )
    }

    const status = statusOf(inv, mode)
    return (
        <div className="flex flex-wrap items-center gap-2 border-b border-border/60 py-2 last:border-0">
            {isTracking(inv) ? <Link2 className="h-4 w-4 shrink-0 text-muted-foreground"/> :
                <Ticket className="h-4 w-4 shrink-0 text-muted-foreground"/>}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs font-medium tracking-wider">{formatInviteCode(inv.code)}</code>
            <Badge variant="secondary"
                   className={cn('h-5 text-[10px]', status.inactive ? 'bg-muted text-muted-foreground' : 'bg-primary/15 text-primary')}>
                {status.label}
            </Badge>
            {inv.expires_at && !status.inactive && (
                <span className="text-xs text-muted-foreground">expires {new Date(inv.expires_at).toLocaleDateString()}</span>
            )}
            {showPin && inv.instance_pin && <span className="text-xs text-muted-foreground">→ {inv.instance_pin}</span>}
            <div className="ml-auto flex items-center gap-0.5">
                <CopyButton text={inviteLink(inv.code)} label="Copy link"/>
                <ConfirmDialog
                    title="Revoke this link?"
                    description="It will stop working immediately."
                    confirmLabel="Revoke"
                    destructive
                    onConfirm={() => onRevoke(inv.code)}
                    trigger={
                        <Button variant="ghost" size="icon" className="h-7 w-7 text-muted-foreground hover:text-destructive" title="Revoke">
                            <Trash2 className="h-3.5 w-3.5"/>
                        </Button>
                    }
                />
            </div>
        </div>
    )
}

export interface MintBody {
    max_uses: number | null
    expires_in_days: number | null
    instance_pin?: string | null
}

export interface InvitesManagerProps {
    invites: Invite[] | undefined
    isLoading: boolean
    /** The effective registration mode (drives the mint form). */
    mode: RegistrationMode | undefined
    /** Whether the current viewer may mint under the mode (mode + role). */
    canMint: boolean
    onMint: (body: MintBody) => Promise<Invite>
    onRevoke: (code: string) => Promise<void>
    /** Resolver-only: offer an instance-pin picker + show the pin column. */
    pinOptions?: string[]
    showPin?: boolean
    /** Admin/fleet view — group the list by the user who created each invite. */
    groupByCreator?: boolean
}

export function InvitesManager({invites, isLoading, mode, canMint, onMint, onRevoke, pinOptions, showPin, groupByCreator}: InvitesManagerProps) {
    const open = mode === 'open'
    const [uses, setUses] = useState('1')
    const [unlimited, setUnlimited] = useState(false)
    const [expiryDays, setExpiryDays] = useState('')
    const [pin, setPin] = useState('__any__')
    const [minting, setMinting] = useState(false)

    // In open mode each user gets a single referral link — once one exists, don't offer to mint another.
    // (Only when viewing one's own list; a grouped admin/fleet list mixes creators.)
    const existingReferral = open && !pinOptions && !groupByCreator ? invites?.find(isTracking) : undefined
    const canCreate = canMint && !existingReferral

    // Group by creator for the admin/fleet view.
    const grouped = groupByCreator
        ? [...(invites ?? []).reduce((m, inv) => {
            const g = m.get(inv.created_by) ?? []
            g.push(inv)
            m.set(inv.created_by, g)
            return m
        }, new Map<string, Invite[]>()).entries()].sort(([a], [b]) => a.localeCompare(b))
        : null

    const mint = async () => {
        setMinting(true)
        try {
            // open ⇒ tracking referral (null); else a capped/uncapped invitation.
            const max_uses = open ? null : unlimited ? 0 : Math.max(1, Math.round(Number(uses) || 1))
            const expires_in_days = !open && expiryDays.trim() ? Math.max(1, Math.round(Number(expiryDays))) : null
            const instance_pin = pinOptions && pin !== '__any__' ? pin : null
            const inv = await onMint({max_uses, expires_in_days, instance_pin})
            await navigator.clipboard.writeText(inviteLink(inv.code)).catch(() => undefined)
            toast.success(open ? 'Referral link ready — copied' : 'Invite created — link copied')
        } catch (e) {
            toast.error('Could not create', {description: apiErrorMessage(e)})
        } finally {
            setMinting(false)
        }
    }

    return (
        <div className="space-y-4">
            {canCreate ? (
                open ? (
                    <div className="flex flex-wrap items-center gap-3 rounded-lg border border-border bg-muted/20 p-3">
                        <p className="flex-1 text-xs text-muted-foreground">
                            Share a referral link — anyone can join and we'll record they came from you.
                        </p>
                        <Button size="sm" className="h-8 gap-1.5" onClick={mint} disabled={minting}>
                            {minting ? <Loader2 className="h-4 w-4 animate-spin"/> : <Plus className="h-4 w-4"/>}
                            Create referral link
                        </Button>
                    </div>
                ) : (
                    <div className="flex flex-wrap items-end gap-3 rounded-lg border border-border bg-muted/20 p-3">
                        <div className="space-y-1.5">
                            <Label className="text-xs">Uses</Label>
                            <div className="flex items-center gap-2">
                                <NumberInput min={1} step={1} value={uses} disabled={unlimited} onChange={(e) => setUses(e.target.value)}
                                             className="h-8 w-20"/>
                                <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
                                    <Checkbox checked={unlimited} onCheckedChange={(c) => setUnlimited(c === true)}/>
                                    Unlimited
                                </label>
                            </div>
                        </div>
                        <div className="space-y-1.5">
                            <Label className="text-xs">Expires (days)</Label>
                            <NumberInput min={1} step={1} placeholder="never" value={expiryDays} onChange={(e) => setExpiryDays(e.target.value)}
                                         className="h-8 w-24"/>
                        </div>
                        {pinOptions && pinOptions.length > 0 && (
                            <div className="space-y-1.5">
                                <Label className="text-xs">Pin to instance</Label>
                                <Select value={pin} onValueChange={setPin}>
                                    <SelectTrigger className="h-8 w-48"><SelectValue/></SelectTrigger>
                                    <SelectContent>
                                        <SelectItem value="__any__">Any (by strategy)</SelectItem>
                                        {pinOptions.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                                    </SelectContent>
                                </Select>
                            </div>
                        )}
                        <Button size="sm" className="h-8 gap-1.5" onClick={mint} disabled={minting}>
                            {minting ? <Loader2 className="h-4 w-4 animate-spin"/> : <Plus className="h-4 w-4"/>}
                            Create invite
                        </Button>
                    </div>
                )
            ) : !canMint ? (
                <p className="text-sm text-muted-foreground">Only an administrator can create invites right now.</p>
            ) : null}

            {isLoading ? (
                <div className="space-y-2">{Array.from({length: 2}).map((_, i) => <Skeleton key={i} className="h-8 w-full"/>)}</div>
            ) : grouped ? (
                grouped.length === 0 ? (
                    <p className="text-sm text-muted-foreground">No invites yet.</p>
                ) : (
                    <div className="space-y-3">
                        {grouped.map(([creator, list]) => (
                            <div key={creator}>
                                <p className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                                    <UserRound className="h-3.5 w-3.5"/> @{creator} <span className="text-muted-foreground/60">· {list.length}</span>
                                </p>
                                <div className="rounded-lg border border-border px-3">
                                    {list.map((inv) => <InviteRow key={inv.code} inv={inv} mode={mode} onRevoke={onRevoke} showPin={showPin}/>)}
                                </div>
                            </div>
                        ))}
                    </div>
                )
            ) : invites && invites.length > 0 ? (
                <div className="rounded-lg border border-border px-3">
                    {[...invites]
                        .sort((a, b) => Number(isStrandedReferral(b, mode)) - Number(isStrandedReferral(a, mode)))
                        .map((inv) => <InviteRow key={inv.code} inv={inv} mode={mode} onRevoke={onRevoke} showPin={showPin}/>)}
                </div>
            ) : (
                <p className="text-sm text-muted-foreground">{open ? 'No referral link yet.' : 'No invites yet.'}</p>
            )}
        </div>
    )
}
