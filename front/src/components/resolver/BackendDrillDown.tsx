import {useMemo, useState} from 'react'
import {AlertTriangle, Save} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {Badge} from '@/components/ui/badge'
import {NumberInput} from '@/components/ui/number-input'
import {AdminClientProvider, proxyAdminClient} from '@/api/adminClient'
import {AdminDashboard} from '@/components/admin/AdminDashboard'
import {relTime, unreachableReason} from '@/components/resolver/BackendHealthList'
import {useResolverCapacityMutation} from '@/hooks/useResolverAdmin'
import {apiErrorMessage} from '@/api/client'
import type {ResolverBackend} from '@/lib/types'

function CapacityEditor({b}: { b: ResolverBackend }) {
    const [accepting, setAccepting] = useState(b.accepting_registrations)
    const [maxUsers, setMaxUsers] = useState(b.max_users == null ? '' : String(b.max_users))
    const mutation = useResolverCapacityMutation()

    const dirty = accepting !== b.accepting_registrations || (b.max_users == null ? '' : String(b.max_users)) !== maxUsers

    const save = async () => {
        try {
            await mutation.mutateAsync({
                backDomain: b.back_domain,
                body: {accepting_registrations: accepting, max_users: maxUsers.trim() === '' ? null : Math.max(0, Math.round(Number(maxUsers)))},
            })
            toast.success('Capacity updated')
        } catch (e) {
            toast.error('Could not update capacity', {description: apiErrorMessage(e)})
        }
    }

    return (
        <div className="flex flex-wrap items-end gap-4 rounded-lg border border-border bg-muted/20 p-3">
            <label className="flex items-center gap-2 text-sm">
                <Switch checked={accepting} onCheckedChange={setAccepting}/>
                Accepting registrations
            </label>
            <div className="space-y-1.5">
                <Label className="text-xs">Max users</Label>
                <NumberInput min={0} step={1} placeholder="unlimited" value={maxUsers} onChange={(e) => setMaxUsers(e.target.value)}
                             className="h-8 w-32"/>
            </div>
            <Button size="sm" className="h-8 gap-1.5" onClick={save} disabled={!dirty || mutation.isPending}>
                <Save className="h-3.5 w-3.5"/> Save
            </Button>
        </div>
    )
}

/** Resolver-side state for a backend (heartbeat, delegation, reachability) — the fleet's own view. */
function ResolverInfo({b}: { b: ResolverBackend }) {
    const rows: [string, React.ReactNode][] = [
        ['Reachable', b.reachable ? <Badge variant="secondary" className="bg-emerald-500/15 text-emerald-500">yes</Badge> :
            <Badge variant="secondary" className="bg-red-500/15 text-red-500">no · {unreachableReason(b)}</Badge>],
        ['Healthy (self-reported)', b.healthy ? 'yes' : 'no'],
        ['Last heartbeat', relTime(b.last_heartbeat_at)],
        ['Delegation expires', b.delegation_expires_at ? new Date(b.delegation_expires_at).toLocaleString() : '—'],
        ['Version', b.version ? `v${b.version}` : '—'],
        ['Accepting registrations', b.accepting_registrations ? 'yes' : 'no'],
        ['Users / max', b.max_users != null ? `${b.user_count.toLocaleString()} / ${b.max_users.toLocaleString()}` : `${b.user_count.toLocaleString()} / ∞`],
        ['Last selected for signup', relTime(b.last_selected_at)],
        ['Uses HTTPS', b.use_https ? 'yes' : 'no'],
        ['Registered', new Date(b.created_at).toLocaleString()],
    ]
    return (
        <dl className="grid grid-cols-1 gap-x-8 gap-y-1.5 rounded-lg border border-border p-4 text-sm sm:grid-cols-2">
            {rows.map(([k, v]) => (
                <div key={k} className="flex items-center justify-between gap-3 border-b border-border/40 pb-1.5 last:border-0">
                    <dt className="text-muted-foreground">{k}</dt>
                    <dd className="text-right">{v}</dd>
                </div>
            ))}
        </dl>
    )
}

/**
 * Full drill-down into a backend's own `/api/admin/*` (feature 24 §5) — the *same* admin dashboard the
 * backend `/admin` renders, but every call proxied through the resolver's delegation replay. Invites
 * are user-auth (not proxied) so that tab is omitted; a resolver-only **Resolver** tab carries the
 * capacity editor + fleet-side backend state so it doesn't crowd the top of the dashboard.
 */
export function BackendDrillDown({b}: { b: ResolverBackend }) {
    const ctx = useMemo(() => ({client: proxyAdminClient(b.back_domain), scope: b.back_domain}), [b.back_domain])

    if (!b.reachable) {
        return (
            <div className="space-y-4">
                <p className="flex items-center gap-2 rounded-md bg-muted/40 px-3 py-2 text-sm text-muted-foreground">
                    <AlertTriangle className="h-4 w-4"/> This backend is unreachable — its admin surface can't be proxied.
                </p>
                <CapacityEditor b={b}/>
                <ResolverInfo b={b}/>
            </div>
        )
    }

    return (
        <AdminClientProvider value={ctx}>
            <AdminDashboard
                showInvites={false}
                extraTabsFirst
                defaultTab="resolver"
                extraTabs={[{
                    value: 'resolver',
                    label: 'Resolver',
                    content: <div className="space-y-4"><CapacityEditor b={b}/><ResolverInfo b={b}/></div>,
                }]}
            />
        </AdminClientProvider>
    )
}
