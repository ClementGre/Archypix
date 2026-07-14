import {useEffect, useState} from 'react'
import {useNavigate} from 'react-router-dom'
import {HardDrive, Loader2, Ticket, Trash2, UserCog, Users} from 'lucide-react'
import {toast} from 'sonner'
import {Card, CardContent, CardHeader, CardTitle} from '@/components/ui/card'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Button} from '@/components/ui/button'
import {RadioGroup, RadioGroupItem} from '@/components/ui/radio-group'
import {Skeleton} from '@/components/ui/skeleton'
import {NumberInput} from '@/components/ui/number-input'
import {STORAGE_SEGMENT_CLASS, StorageBar} from '@/components/StorageBar'
import {InvitesManager} from '@/components/admin/InvitesManager'
import {useAuthStore} from '@/stores/auth'
import {apiErrorMessage} from '@/api/client'
import {useSettings, useStorage, useUpdateProfile, useUpdateSettings} from '@/hooks/useSettings'
import {useInvitations, useInviteMutations, useInvites, useRegistrationInfo} from '@/hooks/useInvites'
import {cn, formatBytes} from '@/lib/utils'
import type {VersioningMode} from '@/lib/types'

// ---------- Account (profile + library) — one form, explicit Save ----------

const VERSIONING_OPTIONS: { value: VersioningMode; label: string; description: string }[] = [
    {value: 'none', label: 'No versioning', description: 'Never keep previous versions.'},
    {value: 'original_copy', label: 'Original copy', description: 'Snapshot the original once, on first edit.'},
    {value: 'full_versioning', label: 'Full versioning', description: 'Snapshot before every visual edit.'},
]

function AccountCard() {
    const user = useAuthStore((s) => s.user)
    const instance = useAuthStore((s) => s.instance)
    const {data: settings, isLoading} = useSettings()
    const updateProfile = useUpdateProfile()
    const updateSettings = useUpdateSettings()

    // Local draft, seeded from server state; committed only on Save (no autosave).
    const [displayName, setDisplayName] = useState('')
    const [email, setEmail] = useState('')
    const [versioning, setVersioning] = useState<VersioningMode>('none')
    const [retention, setRetention] = useState(30)

    useEffect(() => {
        if (user) {
            setDisplayName(user.display_name)
            setEmail(user.email)
        }
    }, [user])
    useEffect(() => {
        if (settings) {
            setVersioning(settings.versioning_mode)
            setRetention(settings.trash_retention_days)
        }
    }, [settings])

    const profileDirty = !!user && (displayName !== user.display_name || email !== user.email)
    const settingsDirty = !!settings && (versioning !== settings.versioning_mode || retention !== settings.trash_retention_days)
    const dirty = profileDirty || settingsDirty
    const busy = updateProfile.isPending || updateSettings.isPending

    const save = async () => {
        if (!displayName.trim()) return toast.error('Display name is required')
        if (!/.+@.+\..+/.test(email)) return toast.error('A valid email is required')
        if (retention < 1 || retention > 3650) return toast.error('Retention must be between 1 and 3650 days')
        try {
            if (profileDirty) await updateProfile.mutateAsync({display_name: displayName.trim(), email: email.trim()})
            if (settingsDirty) await updateSettings.mutateAsync({versioning_mode: versioning, trash_retention_days: retention})
            toast.success('Changes saved')
        } catch (e) {
            toast.error('Could not save changes', {description: apiErrorMessage(e)})
        }
    }

    const reset = () => {
        if (user) {
            setDisplayName(user.display_name)
            setEmail(user.email)
        }
        if (settings) {
            setVersioning(settings.versioning_mode)
            setRetention(settings.trash_retention_days)
        }
    }

    return (
        <Card>
            <CardHeader>
                <CardTitle className="flex items-center gap-2">
                    <UserCog className="h-4 w-4"/>
                    Account
                </CardTitle>
            </CardHeader>
            <CardContent className="space-y-6">
                <div className="text-sm text-muted-foreground">
                    <span className="font-mono">@{user?.username ?? '…'}:{instance ?? '…'}</span>
                </div>

                <div className="grid gap-4 sm:grid-cols-2">
                    <div className="space-y-1.5">
                        <Label htmlFor="display_name">Display name</Label>
                        <Input id="display_name" value={displayName} onChange={(e) => setDisplayName(e.target.value)}/>
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="email">Email</Label>
                        <Input id="email" type="email" value={email} onChange={(e) => setEmail(e.target.value)}/>
                    </div>
                </div>

                <div className="space-y-3">
                    <Label>Versioning mode</Label>
                    {isLoading ? (
                        <div className="space-y-2">{Array.from({length: 3}).map((_, i) => <Skeleton key={i} className="h-5 w-56"/>)}</div>
                    ) : (
                        <RadioGroup value={versioning} onValueChange={(v) => setVersioning(v as VersioningMode)}>
                            {VERSIONING_OPTIONS.map((opt) => (
                                <div key={opt.value} className="flex items-start gap-3">
                                    <RadioGroupItem value={opt.value} id={`versioning-${opt.value}`} className="mt-0.5"/>
                                    <div>
                                        <Label htmlFor={`versioning-${opt.value}`} className="font-medium">{opt.label}</Label>
                                        <p className="text-xs text-muted-foreground">{opt.description}</p>
                                    </div>
                                </div>
                            ))}
                        </RadioGroup>
                    )}
                </div>

                <div className="space-y-1.5">
                    <Label htmlFor="trash-retention">Trash retention</Label>
                    <div className="flex items-center gap-2">
                        <NumberInput
                            id="trash-retention"
                            className="h-10 w-28"
                            min={1}
                            max={3650}
                            step={1}
                            value={retention}
                            onChange={(e) => setRetention(Math.round(Number(e.target.value) || 0))}
                        />
                        <span className="text-sm text-muted-foreground">days</span>
                    </div>
                    <p className="text-xs text-muted-foreground">
                        How long your trashed photos are kept before being permanently deleted (1–3650).
                    </p>
                </div>

                <div className="flex items-center gap-2">
                    <Button onClick={save} disabled={!dirty || busy}>
                        {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                        Save changes
                    </Button>
                    {dirty && (
                        <Button variant="ghost" onClick={reset} disabled={busy}>Discard</Button>
                    )}
                </div>
            </CardContent>
        </Card>
    )
}

// ---------- Storage (feature 22) ----------

function formatUsage(bytes: number): string {
    return bytes === 0 ? '0 KB' : formatBytes(bytes)
}

function BreakdownRow({label, bytes, swatchClassName}: { label: string; bytes: number; swatchClassName: string }) {
    return (
        <div className="flex items-center justify-between text-sm">
            <span className="flex items-center gap-2 text-muted-foreground">
                <span className={cn('h-2.5 w-2.5 shrink-0 rounded-sm', swatchClassName)}/>
                {label}
            </span>
            <span className="tabular-nums">{formatUsage(bytes)}</span>
        </div>
    )
}

function StorageCard() {
    const navigate = useNavigate()
    const {data: storage, isLoading} = useStorage()

    if (isLoading || !storage) {
        return (
            <Card>
                <CardHeader><CardTitle>Storage</CardTitle></CardHeader>
                <CardContent><Skeleton className="h-24 w-full"/></CardContent>
            </Card>
        )
    }

    const {quota_bytes, used_bytes, breakdown, reclaimable_trash_bytes, usage_ratio, warn_level} = storage
    const pct = quota_bytes && quota_bytes > 0 ? Math.min(100, Math.round((usage_ratio ?? 0) * 100)) : 0

    return (
        <Card>
            <CardHeader>
                <CardTitle className="flex items-center gap-2"><HardDrive className="h-4 w-4"/>Storage</CardTitle>
            </CardHeader>
            <CardContent className="space-y-5">
                <div className="space-y-1.5">
                    <div className="flex items-center justify-between text-sm">
                        <span>
                            <span className="font-medium tabular-nums">{formatUsage(used_bytes)}</span>
                            {quota_bytes && quota_bytes > 0 ? (
                                <span className="text-muted-foreground"> of {formatBytes(quota_bytes)} used</span>
                            ) : (
                                <span className="text-muted-foreground"> used · unlimited</span>
                            )}
                        </span>
                        {quota_bytes && quota_bytes > 0 && <span className="tabular-nums text-muted-foreground">{pct}%</span>}
                    </div>
                    <StorageBar breakdown={breakdown} quotaBytes={quota_bytes} usedBytes={used_bytes} className="rounded-full"/>
                    {warn_level === 'full' && (
                        <p className="text-xs text-destructive">Storage is full. free up space (or empty your trash) before uploading more.</p>
                    )}
                    {warn_level === 'critical' && (
                        <p className="text-xs text-amber-600 dark:text-amber-500">You are almost out of space.</p>
                    )}
                </div>

                <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2 sm:gap-x-8">
                    <BreakdownRow label="Originals" bytes={breakdown.originals_bytes} swatchClassName={STORAGE_SEGMENT_CLASS.originals}/>
                    <BreakdownRow label="Versions" bytes={breakdown.versions_bytes} swatchClassName={STORAGE_SEGMENT_CLASS.versions}/>
                    <BreakdownRow label="Trashed originals" bytes={breakdown.originals_trashed_bytes}
                                  swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                    <BreakdownRow label="Trashed versions" bytes={breakdown.versions_trashed_bytes} swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                </div>

                {reclaimable_trash_bytes > 0 && (
                    <div className="flex items-center justify-between rounded-md border border-border bg-muted/30 px-3 py-2">
                        <span className="text-sm text-muted-foreground">
                            Empty your trash to reclaim{' '}
                            <span className="font-medium text-foreground">{formatBytes(reclaimable_trash_bytes)}</span>
                        </span>
                        <Button variant="outline" size="sm" className="h-7 gap-1.5" onClick={() => navigate('/?trash=only')}>
                            <Trash2 className="h-3.5 w-3.5"/>Open trash
                        </Button>
                    </div>
                )}
            </CardContent>
        </Card>
    )
}

// ---------- Invites + invitation graph (feature 23 §6) ----------

function InvitesCard() {
    const {data: invites, isLoading} = useInvites()
    const {data: info} = useRegistrationInfo()
    const {data: graph} = useInvitations()
    const {mint, revoke} = useInviteMutations()
    const isAdmin = useAuthStore((s) => s.user?.is_admin ?? false)
    const canMint = info ? info.mode !== 'admin_invite' || isAdmin : false
    const open = info?.mode === 'open'

    return (
        <Card>
            <CardHeader>
                <CardTitle className="flex items-center gap-2">
                    <Ticket className="h-4 w-4"/>
                    {open ? 'Referral link' : 'Invites'}
                </CardTitle>
            </CardHeader>
            <CardContent className="space-y-5">
                <InvitesManager
                    invites={invites}
                    isLoading={isLoading}
                    mode={info?.mode}
                    canMint={canMint}
                    onMint={(body) => mint.mutateAsync({max_uses: body.max_uses, expires_in_days: body.expires_in_days})}
                    onRevoke={async (code) => {
                        try {
                            await revoke.mutateAsync(code)
                            toast.success('Revoked')
                        } catch (e) {
                            toast.error('Could not revoke', {description: apiErrorMessage(e)})
                        }
                    }}
                />

                {/* Invited people — shown in the same card. */}
                {graph && (graph.invited_by || graph.invited.length > 0) && (
                    <div className="space-y-2 border-t border-border/60 pt-4 text-sm">
                        {graph.invited_by && (
                            <p className="flex items-center gap-1.5 text-muted-foreground">
                                <Users className="h-4 w-4"/> Invited by <span className="font-medium text-foreground">@{graph.invited_by}</span>
                            </p>
                        )}
                        {graph.invited.length > 0 && (
                            <div>
                                <p className="mb-1 text-muted-foreground">You brought
                                    in {graph.invited.length} {graph.invited.length === 1 ? 'person' : 'people'}:</p>
                                <div className="flex flex-wrap gap-1.5">
                                    {graph.invited.map((u) => <span key={u} className="rounded-full bg-muted px-2 py-0.5 text-xs">@{u}</span>)}
                                </div>
                            </div>
                        )}
                    </div>
                )}
            </CardContent>
        </Card>
    )
}

// ---------- Page ----------

export default function SettingsPage() {
    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-2xl space-y-6">
                <h1 className="text-xl font-semibold">Profile</h1>
                <AccountCard/>
                <StorageCard/>
                <InvitesCard/>
            </div>
        </div>
    )
}
