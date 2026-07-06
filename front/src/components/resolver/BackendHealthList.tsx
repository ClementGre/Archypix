import {Clock} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {formatBytes} from '@/lib/utils'
import type {ResolverBackend} from '@/lib/types'

export function relTime(iso: string | null): string {
    if (!iso) return 'never'
    const diff = Date.now() - new Date(iso).getTime()
    const s = Math.floor(diff / 1000)
    if (s < 60) return `${s}s ago`
    const m = Math.floor(s / 60)
    if (m < 60) return `${m}m ago`
    const h = Math.floor(m / 60)
    if (h < 24) return `${h}h ago`
    return `${Math.floor(h / 24)}d ago`
}

/** Why a backend is unreachable — surfaced so the operator knows what to fix (feature 24 §5). */
export function unreachableReason(b: ResolverBackend): string {
    if (!b.last_heartbeat_at) return 'never sent a heartbeat'
    if (b.delegation_expires_at && new Date(b.delegation_expires_at).getTime() < Date.now()) return 'delegation token expired'
    return 'no recent heartbeat'
}

function Dot({ok}: { ok: boolean }) {
    return <span className={`inline-block h-2 w-2 rounded-full ${ok ? 'bg-emerald-500' : 'bg-red-500'}`}/>
}

export function BackendHealthRow({b, children}: { b: ResolverBackend; children?: React.ReactNode }) {
    const pct = b.max_users && b.max_users > 0 ? Math.min(100, Math.round((b.user_count / b.max_users) * 100)) : null
    const capacityLabel = b.max_users != null
        ? `${b.user_count.toLocaleString()} / ${b.max_users.toLocaleString()}${pct != null ? ` (${pct}%)` : ''}`
        : b.user_count.toLocaleString()
    return (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-border/60 py-2.5 last:border-0">
            <div className="flex min-w-0 items-center gap-2">
                <Dot ok={b.reachable}/>
                <span className="truncate font-mono text-sm">{b.back_domain}</span>
                {b.version && <span className="text-[10px] text-muted-foreground">v{b.version}</span>}
            </div>
            {!b.reachable && (
                <Badge variant="secondary" className="h-5 bg-red-500/15 text-[10px] text-red-500">
                    unreachable · {unreachableReason(b)}
                </Badge>
            )}
            {b.reachable && !b.accepting_registrations && (
                <Badge variant="secondary" className="h-5 bg-amber-500/15 text-[10px] text-amber-600 dark:text-amber-500">closed</Badge>
            )}
            <div className="ml-auto flex items-center gap-4 text-xs text-muted-foreground">
                <span title="Users">{capacityLabel} users</span>
                <span title="Pictures">{b.picture_count.toLocaleString()} pics</span>
                <span title="Storage">{formatBytes(b.storage_bytes)}</span>
                <span className="flex items-center gap-1"><Clock className="h-3 w-3"/>{relTime(b.last_heartbeat_at)}</span>
                {children}
            </div>
        </div>
    )
}

export function BackendHealthList({backends}: { backends: ResolverBackend[] }) {
    if (backends.length === 0) {
        return <p className="text-sm text-muted-foreground">No backends have registered yet.</p>
    }
    return (
        <div className="rounded-lg border border-border px-4">
            {backends.map((b) => <BackendHealthRow key={b.back_domain} b={b}/>)}
        </div>
    )
}
