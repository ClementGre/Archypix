import {useMemo} from 'react'
import {Server} from 'lucide-react'
import {Skeleton} from '@/components/ui/skeleton'
import {AdminClientProvider, proxyAdminClient} from '@/api/adminClient'
import {UsersTab} from '@/components/admin/UsersTab'
import {useResolverBackends} from '@/hooks/useResolverAdmin'
import type {ResolverBackend} from '@/lib/types'

/** One backend's full user-management table, proxied to that backend (feature 24). */
function BackendUsers({b}: { b: ResolverBackend }) {
    const ctx = useMemo(() => ({client: proxyAdminClient(b.back_domain), scope: b.back_domain}), [b.back_domain])
    return (
        <section className="space-y-2">
            <h3 className="flex items-center gap-1.5 text-sm font-semibold">
                <Server className="h-4 w-4 text-muted-foreground"/>
                {b.back_domain}
            </h3>
            <AdminClientProvider value={ctx}>
                {/* Reuses the backend `/admin` UsersTab verbatim — same info + options, routed to this backend. */}
                <UsersTab showCreate/>
            </AdminClientProvider>
        </section>
    )
}

/**
 * Fleet users (feature 24) — reuses the backend `UsersTab` per reachable backend (create/edit/quota/
 * delete/audit all route to the right instance via the resolver proxy), grouped by backend.
 */
export function ResolverUsersTab({refetchInterval}: { refetchInterval: number | false }) {
    const {data: backends, isLoading} = useResolverBackends({refetchInterval})
    const reachable = (backends ?? []).filter((b) => b.reachable)

    if (isLoading) {
        return <div className="space-y-2">{Array.from({length: 3}).map((_, i) => <Skeleton key={i} className="h-24 w-full"/>)}</div>
    }
    if (reachable.length === 0) {
        return <p className="text-sm text-muted-foreground">No reachable backends.</p>
    }
    return (
        <div className="space-y-8">
            {reachable.map((b) => <BackendUsers key={b.back_domain} b={b}/>)}
        </div>
    )
}
