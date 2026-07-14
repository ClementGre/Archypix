import {type ReactNode, useState} from 'react'
import {Check, Info, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {cn, formatDateTime, TagPath} from '@/lib/utils'
import type {ShareStatus} from '@/lib/types'
import {ShareStatusBadge} from './ShareStatusBadge'

export interface ShareInfoEntry {
    /** Optional per-entry label (e.g. the recipient handle for a grouped row). */
    label?: string
    name: string
    message: string | null
    status?: ShareStatus
    /** ShareBack allowed (sender's setting). */
    allowShareBack?: boolean
    /** Recipients may propose EXIF edits the owner auto-applies. */
    allowExifEdit?: boolean
    /** New pictures auto-announced. */
    future?: boolean
    /** Tag the pictures land under (wire form); shown in display form. */
    sharedTag?: string | null
    /** ISO creation/received timestamp. */
    createdAt?: string | null
    /** ISO timestamp of the last picture announcement received (incoming shares only). */
    lastReceivedAt?: string | null
    /** ISO timestamp of the last failed announcement (outgoing shares only). */
    lastErrorAt?: string | null
    /** ISO timestamp of the next scheduled retry (outgoing shares only). */
    nextRetryAt?: string | null
    /** ISO timestamp at which the share was closed (revoked/rejected). */
    closedAt?: string | null
    /** Human label of the share this one is a ShareBack of, or null. */
    sharebackOf?: string | null
}

/** Compact on/off feature chip — consistent styling regardless of side. */
export function FlagChip({label, on}: { label: string; on: boolean }) {
    return (
        <span
            className={cn(
                'inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium',
                on ? 'bg-emerald-500/15 text-emerald-400' : 'bg-muted text-muted-foreground',
            )}
        >
            {on ? <Check className="h-3 w-3"/> : <X className="h-3 w-3"/>}
            {label}
        </span>
    )
}

export function DetailRow({label, children}: { label: string; children: ReactNode }) {
    return (
        <div className="flex items-baseline justify-between gap-3">
            <span className="shrink-0 text-[11px] text-muted-foreground">{label}</span>
            <span className="min-w-0 break-words text-right text-[11px] text-foreground">{children}</span>
        </div>
    )
}

/** Label for the close timestamp depending on how the share ended. */
function closedLabel(status?: ShareStatus): string {
    if (status === 'revoked') return 'Revoked'
    if (status === 'tombstoned') return 'Rejected'
    return 'Closed'
}

function Entry({entry}: { entry: ShareInfoEntry }) {
    const flags: ReactNode[] = []
    if (entry.allowShareBack !== undefined) flags.push(<FlagChip key="sb" label="ShareBack" on={entry.allowShareBack}/>)
    if (entry.allowExifEdit !== undefined) flags.push(<FlagChip key="ee" label="EXIF editing" on={entry.allowExifEdit}/>)
    if (entry.future !== undefined) flags.push(<FlagChip key="fu" label="Future additions" on={entry.future}/>)

    return (
        <div className="space-y-1.5 border-border [&:not(:first-child)]:border-t [&:not(:first-child)]:pt-2">
            {entry.label && <p className="truncate text-[11px] text-muted-foreground">{entry.label}</p>}
            <div className="flex items-center justify-between gap-2">
                <p className="min-w-0 break-words text-sm font-medium">{entry.name}</p>
                {entry.status && <ShareStatusBadge status={entry.status}/>}
            </div>

            {flags.length > 0 && <div className="flex flex-wrap gap-1">{flags}</div>}

            <div className="space-y-0.5">
                {entry.sharedTag != null && entry.sharedTag !== '' && (
                    <DetailRow label="Shared tag">{TagPath.toDisplay(entry.sharedTag)}</DetailRow>
                )}
                {entry.sharebackOf && <DetailRow label="ShareBack of">{entry.sharebackOf}</DetailRow>}
                {entry.createdAt && <DetailRow label="Created">{formatDateTime(entry.createdAt)}</DetailRow>}
                {entry.lastReceivedAt !== undefined && (
                    <DetailRow label="Last received">
                        {entry.lastReceivedAt ? formatDateTime(entry.lastReceivedAt) : 'Never'}
                    </DetailRow>
                )}
                {entry.lastErrorAt && (
                    <DetailRow label="Last error"><span className="text-red-400">{formatDateTime(entry.lastErrorAt)}</span></DetailRow>
                )}
                {entry.nextRetryAt && <DetailRow label="Next retry">{formatDateTime(entry.nextRetryAt)}</DetailRow>}
                {entry.closedAt && (entry.status === 'revoked' || entry.status === 'tombstoned') && (
                    <DetailRow label={closedLabel(entry.status)}>{formatDateTime(entry.closedAt)}</DetailRow>
                )}
            </div>

            {entry.message ? (
                <p className="whitespace-pre-wrap break-words text-xs text-muted-foreground">{entry.message}</p>
            ) : (
                <p className="text-xs italic text-muted-foreground/60">No message</p>
            )}
        </div>
    )
}

/**
 * Details for one or more shares, surfaced in a popover anchored to the right
 * (towards the pictures pane). Opens on hover (desktop) and on click/tap (touch)
 * via the explicit trigger button. `footer` renders below the entries (e.g. a
 * ShareBack action button).
 */
export function ShareInfoPopover({entries, footer}: { entries: ShareInfoEntry[]; footer?: ReactNode }) {
    const [open, setOpen] = useState(false)
    if (!entries.length) return null

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>
                <Button
                    size="icon"
                    variant="ghost"
                    className="h-6 w-6 text-muted-foreground hover:text-foreground"
                    title="Details"
                    onMouseEnter={() => setOpen(true)}
                >
                    <Info className="h-3.5 w-3.5"/>
                </Button>
            </PopoverTrigger>
            <PopoverContent
                side="right"
                align="start"
                className="max-h-[70vh] w-72 space-y-2 overflow-y-auto p-3"
                onMouseLeave={() => setOpen(false)}
            >
                {entries.map((entry, i) => (
                    <Entry key={i} entry={entry}/>
                ))}
                {footer && <div className="border-t border-border pt-2">{footer}</div>}
            </PopoverContent>
        </Popover>
    )
}

/** The most common `name` among a set of shares, with a "(and N others)" suffix
 *  when the group mixes several distinct names. */
export function summarizeNames(names: string[]): string {
    if (!names.length) return ''
    const counts = new Map<string, number>()
    for (const n of names) counts.set(n, (counts.get(n) ?? 0) + 1)
    const [top] = [...counts.entries()].sort((a, b) => b[1] - a[1])
    const others = names.length - top[1]
    return others > 0 ? `${top[0]} (and ${others} other${others !== 1 ? 's' : ''})` : top[0]
}
