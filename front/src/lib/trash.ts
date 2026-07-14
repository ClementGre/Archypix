import {formatDateTime} from '@/lib/utils'

/** Whole minutes from now until `iso` (negative if already past). */
export function isoToMinDelta(iso: string | null | undefined): number | null {
    if (!iso) return null
    const hasTz = /[zZ]$|[+-]\d{2}:?\d{2}$/.test(iso)
    const normalized = !hasTz && iso.includes('T') ? `${iso}Z` : iso
    const t = new Date(normalized).getTime()
    if (Number.isNaN(t)) return null
    return Math.ceil((t - Date.now()) / 60_000)
}

/** Short human countdown, e.g. "in 12 days", "tomorrow", "today", "overdue". */
export function countdown(iso: string | null | undefined): string {
    let min = isoToMinDelta(iso)
    if (min == null) return ''
    if (min <= 0) return 'Overdue'

    const d = Math.floor(min / 1440)
    if (d >= 1) return `${d} days left`

    const h = Math.floor(min / 60)
    if (h >= 1) return `${Math.floor(h)} hours left`

    if (min >= 0) return `${min} minutes left`
    return `${d} days left`
}

/** Owner purge deadline for an owned trashed picture: deleted_at + retention days. */
export function ownedPurgeAt(deletedAt: string | null | undefined, retentionDays: number): string | null {
    if (!deletedAt) return null
    const hasTz = /[zZ]$|[+-]\d{2}:?\d{2}$/.test(deletedAt)
    const normalized = !hasTz && deletedAt.includes('T') ? `${deletedAt}Z` : deletedAt
    const t = new Date(normalized).getTime()
    if (Number.isNaN(t)) return null
    return new Date(t + retentionDays * 86_400_000).toISOString()
}

/** "Apr 3, 2026 (in 12 days)" style label for a deadline. */
export function deadlineLabel(iso: string | null | undefined): string {
    if (!iso) return '—'
    const c = countdown(iso)
    return `${formatDateTime(iso)}${c ? ` (${c})` : ''}`
}
