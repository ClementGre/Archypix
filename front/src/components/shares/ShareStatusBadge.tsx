import {Badge} from '@/components/ui/badge'
import {cn} from '@/lib/utils'
import type {ShareStatus} from '@/lib/types'

const STATUS: Record<ShareStatus, { label: string; className: string }> = {
    pending: {label: 'Pending', className: 'bg-amber-500/15 text-amber-500'},
    pending_first_announcement: {label: 'Delivering', className: 'bg-sky-500/15 text-sky-400'},
    active: {label: 'Active', className: 'bg-emerald-500/15 text-emerald-500'},
    errored: {label: 'Error', className: 'bg-red-500/15 text-red-500'},
    revoked: {label: 'Revoked', className: 'bg-zinc-500/15 text-zinc-400'},
    tombstoned: {label: 'Rejected', className: 'bg-zinc-500/15 text-zinc-400'},
}

export function ShareStatusBadge({status}: { status: ShareStatus }) {
    const s = STATUS[status]
    return (
        <Badge variant="secondary" className={cn('border-0 font-medium', s.className)}>
            {s.label}
        </Badge>
    )
}
