import {cn} from '@/lib/utils'
import type {StorageBreakdown} from '@/lib/types'

/** Segment colors shared between the bar and any legend/breakdown rows next to it. */
export const STORAGE_SEGMENT_CLASS = {
    originals: 'bg-primary',
    versions: 'bg-sky-500',
    trashed: 'bg-muted-foreground/40 bg-diagonal-stripes',
} as const

/**
 * Segmented storage usage bar (feature 22): one stacked segment per billed category
 * (live originals, live versions, trashed — striped since it's still billed but reclaimable).
 * Segment widths are relative to the quota when set, else to `usedBytes` (composition only).
 */
export function StorageBar({
                               breakdown,
                               quotaBytes,
                               usedBytes,
                               className,
                           }: {
    breakdown: StorageBreakdown
    quotaBytes: number | null
    usedBytes: number
    className?: string
}) {
    const total = quotaBytes && quotaBytes > 0 ? quotaBytes : usedBytes
    const pct = (bytes: number) => (total > 0 ? Math.min(100, (bytes / total) * 100) : 0)

    const originalsPct = pct(breakdown.originals_bytes)
    const versionsPct = pct(breakdown.versions_bytes)
    const trashedPct = pct(breakdown.originals_trashed_bytes + breakdown.versions_trashed_bytes)

    return (
        <div className={cn('flex h-2 w-full overflow-hidden rounded-full bg-muted', className)}>
            <div className={cn('h-full', STORAGE_SEGMENT_CLASS.originals)} style={{width: `${originalsPct}%`}}/>
            <div className={cn('h-full', STORAGE_SEGMENT_CLASS.versions)} style={{width: `${versionsPct}%`}}/>
            <div className={cn('h-full', STORAGE_SEGMENT_CLASS.trashed)} style={{width: `${trashedPct}%`}}/>
        </div>
    )
}
