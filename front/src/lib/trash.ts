import {formatDateTime} from '@/lib/utils'

/** Whole days from now until `iso` (negative if already past). */
export function daysUntil(iso: string | null | undefined): number | null {
    if (!iso) return null
    const hasTz = /[zZ]$|[+-]\d{2}:?\d{2}$/.test(iso)
    const normalized = !hasTz && iso.includes('T') ? `${iso}Z` : iso
    const t = new Date(normalized).getTime()
    if (Number.isNaN(t)) return null
    return Math.ceil((t - Date.now()) / 86_400_000)
}

/** Short human countdown, e.g. "in 12 days", "tomorrow", "today", "overdue". */
export function countdown(iso: string | null | undefined): string {
    const d = daysUntil(iso)
    if (d == null) return ''
    if (d < 0) return 'overdue'
    if (d === 0) return 'today'
    if (d === 1) return 'tomorrow'
    return `in ${d} days`
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
