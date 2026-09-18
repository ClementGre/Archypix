// Grouping sections (feature 35 §4). Grouping buckets the **sort field**, so a section is always a
// contiguous run in the sorted order — which is why sections are cut by walking the ordered items
// and starting a new one whenever the bucket key changes, rather than by grouping into a map.
//
// Every field gets a terminal **null bucket** (*No date*, *No filename*, *Unknown size*): without it
// a third of an imported camera roll silently falls out of every section.

import type {
    Grouping,
    GroupingField,
    GroupingKind,
    Hemisphere,
    PictureListItem,
    SortField,
    TagMeta,
} from '@/lib/types'

export type {GroupingKind}

/** The sort fields that carry a per-tag date range, so `in_sections` can place a child (34 §3.4). */
export const DATE_FIELDS = ['captured_at', 'ingested_at', 'updated_at'] as const

export function isDateField(field: SortField): field is (typeof DATE_FIELDS)[number] {
    return (DATE_FIELDS as readonly string[]).includes(field)
}

/** Which bucket kinds a field accepts — mirrors the server's strict validation (34 §3.2). */
export function kindsFor(field: SortField): GroupingKind['kind'][] {
    if (isDateField(field)) return ['none', 'year', 'quarter', 'season', 'month']
    if (field === 'filename') return ['none', 'prefix']
    return ['none', 'magnitude']
}

// Shared instances: the resolved kind feeds section memos, so a fresh object per render would
// invalidate them every time and re-register every section in a loop.
export const NO_GROUPING: GroupingKind = {kind: 'none'}
const MONTH_GROUPING: GroupingKind = {kind: 'month'}

/** A missing key means that sort field's default: `month` for dates, `none` for everything else. */
export function resolveGrouping(meta: TagMeta | null | undefined, field: SortField): GroupingKind {
    const stored = meta?.grouping?.[field as GroupingField]
    if (stored && kindsFor(field).includes(stored.kind)) return stored
    return isDateField(field) ? MONTH_GROUPING : NO_GROUPING
}

/** Whether a grouping actually cuts the stream — `none` renders one unheaded section. */
export function isGrouped(kind: GroupingKind): boolean {
    return kind.kind !== 'none'
}

/** Write a field's bucket without losing the others — buckets are remembered per sort field (§4). */
export function withGrouping(current: Grouping, field: SortField, kind: GroupingKind): Grouping {
    return {...current, [field as GroupingField]: kind}
}

export interface Bucket {
    /** Identity of the run; a change ends the section. */
    key: string
    label: string
}

/** Context the ladders need beyond the item itself. */
export interface BucketContext {
    /** Season buckets read the **viewer's** convention, whatever the photo's own hemisphere (34 §3). */
    hemisphere: Hemisphere
    /** `time_near`'s reference instant — the ladder buckets `|captured_at − near_time|`. */
    nearTime?: string | null
}

const NO_DATE: Bucket = {key: '~null', label: 'No date'}
const NO_NAME: Bucket = {key: '~null', label: 'No filename'}
const NO_SIZE: Bucket = {key: '~null', label: 'Unknown size'}
const NO_VALUE: Bucket = {key: '~null', label: 'Unknown'}

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December']

/** Northern names shifted two quarters for a southern viewer. */
const SEASONS = ['Winter', 'Spring', 'Summer', 'Autumn']

/** `run` is the calendar quarter (Dec–Feb = 0, …) — the contiguous slice; only the *name* flips. */
function seasonOf(month: number, hemisphere: Hemisphere): { run: number; name: string } {
    const run = Math.floor(((month % 12) + 1) / 3) % 4
    return {run, name: SEASONS[hemisphere === 'south' ? (run + 2) % 4 : run]}
}

/** A naive `YYYY-MM-DD…` timestamp, read without a timezone shift (the wire format is naive UTC). */
function parts(iso: string): { y: number; m: number } | null {
    const m = /^(\d{4})-(\d{2})/.exec(iso)
    return m ? {y: Number(m[1]), m: Number(m[2]) - 1} : null
}

function dateBucket(iso: string | null, kind: GroupingKind['kind'], ctx: BucketContext): Bucket {
    if (!iso) return NO_DATE
    const p = parts(iso)
    if (!p) return NO_DATE
    switch (kind) {
        case 'year':
            return {key: `${p.y}`, label: `${p.y}`}
        case 'quarter': {
            const q = Math.floor(p.m / 3) + 1
            return {key: `${p.y}-Q${q}`, label: `Q${q} ${p.y}`}
        }
        // Keys are zero-padded so they sort lexicographically — `mergeSections` relies on it.
        case 'season': {
            const {run, name} = seasonOf(p.m, ctx.hemisphere)
            // December belongs to the season-year that follows it, so the run stays contiguous.
            const year = run === 0 && p.m === 11 ? p.y + 1 : p.y
            return {key: `${year}-S${run}`, label: `${name} ${year}`}
        }
        case 'month':
        default:
            return {key: `${p.y}-${String(p.m + 1).padStart(2, '0')}`, label: `${MONTHS[p.m]} ${p.y}`}
    }
}

/** One ladder per bucketable field, so a new field ships by declaring its own (§4). */
interface Rung {
    /** Exclusive upper bound; the last rung is `Infinity`. */
    max: number
    label: string
}

const BYTE_LADDER: Rung[] = [
    {max: 100e3, label: 'Under 100 KB'},
    {max: 1e6, label: '100 KB – 1 MB'},
    {max: 10e6, label: '1 – 10 MB'},
    {max: 100e6, label: '10 – 100 MB'},
    {max: 1e9, label: '100 MB – 1 GB'},
    {max: Infinity, label: 'Over 1 GB'},
]

const DISTANCE_LADDER: Rung[] = [
    {max: 10, label: 'Within 10 m'},
    {max: 100, label: '10 – 100 m'},
    {max: 1e3, label: '100 m – 1 km'},
    {max: 10e3, label: '1 – 10 km'},
    {max: 100e3, label: '10 – 100 km'},
    {max: 1e6, label: '100 – 1000 km'},
    {max: Infinity, label: 'Over 1000 km'},
]

const HOUR = 3600e3
const TIME_LADDER: Rung[] = [
    {max: HOUR, label: 'Within an hour'},
    {max: 6 * HOUR, label: '1 – 6 hours'},
    {max: 24 * HOUR, label: '6 – 24 hours'},
    {max: 7 * 24 * HOUR, label: '1 – 7 days'},
    {max: 28 * 24 * HOUR, label: '1 – 4 weeks'},
    {max: 365 * 24 * HOUR, label: '1 – 12 months'},
    {max: Infinity, label: 'Over a year'},
]

function rungBucket(value: number | null | undefined, ladder: Rung[], nullBucket: Bucket): Bucket {
    if (value == null || Number.isNaN(value)) return nullBucket
    const i = ladder.findIndex((r) => value < r.max)
    const rung = ladder[i === -1 ? ladder.length - 1 : i]
    return {key: `m${i}`, label: rung.label}
}

/** Both sides are naive, so parsing them the same way is all a delta needs. */
function naiveMs(iso: string | null | undefined): number | null {
    if (!iso) return null
    const t = Date.parse(iso.replace(' ', 'T'))
    return Number.isNaN(t) ? null : t
}

/** The bucket one item falls into under `field`/`kind`. */
export function bucketOf(
    item: PictureListItem,
    field: SortField,
    kind: GroupingKind,
    ctx: BucketContext,
): Bucket {
    if (kind.kind === 'none') return {key: '', label: ''}
    if (isDateField(field)) {
        const iso = field === 'captured_at' ? item.captured_at
            : field === 'ingested_at' ? item.ingested_at
                : item.updated_at
        return dateBucket(iso, kind.kind, ctx)
    }
    if (field === 'filename') {
        const name = item.filename?.trim()
        if (!name) return NO_NAME
        const chars = kind.kind === 'prefix' ? Math.min(16, Math.max(1, kind.chars)) : 3
        const prefix = name.slice(0, chars)
        return {key: prefix.toLowerCase(), label: prefix}
    }
    if (field === 'file_size') return rungBucket(item.file_size, BYTE_LADDER, NO_SIZE)
    if (field === 'geo_near') return rungBucket(item.distance_m, DISTANCE_LADDER, NO_VALUE)
    if (field === 'time_near') {
        const ref = naiveMs(ctx.nearTime ?? null)
        const at = naiveMs(item.captured_at)
        // Buckets the **absolute** delta, so "3 h before" and "3 h after" share a section (§4).
        return rungBucket(ref == null || at == null ? null : Math.abs(at - ref), TIME_LADDER, NO_VALUE)
    }
    return {key: '', label: ''}
}

export interface Section<T> {
    key: string
    label: string
    items: T[]
}

/**
 * Cut an already-sorted list into sections. A bucket is a contiguous run by construction, so a run
 * that reappears later (which the sort should prevent) simply becomes a second section rather than
 * silently merging rows that are far apart in the order.
 */
export function sectionize(
    items: PictureListItem[],
    field: SortField,
    kind: GroupingKind,
    ctx: BucketContext,
): Section<PictureListItem>[] {
    if (!isGrouped(kind)) return items.length ? [{key: '', label: '', items}] : []
    const out: Section<PictureListItem>[] = []
    for (const item of items) {
        const b = bucketOf(item, field, kind, ctx)
        const last = out[out.length - 1]
        if (last && last.key === b.key) last.items.push(item)
        else out.push({key: b.key, label: b.label, items: [item]})
    }
    // Two runs of the same bucket would collide as React keys.
    const seen = new Map<string, number>()
    return out.map((s) => {
        const n = (seen.get(s.key) ?? 0) + 1
        seen.set(s.key, n)
        return n > 1 ? {...s, key: `${s.key}#${n}`} : s
    })
}

/** The section a tag's date range places it in under `in_sections` (§5). */
export function dateBucketFor(iso: string | null, kind: GroupingKind, ctx: BucketContext): Bucket {
    return dateBucket(iso, kind.kind, ctx)
}

/**
 * Add a section for every child bucket the direct photos don't already cover, in sort order. §5
 * places each child in "the section its date falls into" — a child dated in a month T has no photos
 * in would otherwise have nowhere to render. Date bucket keys are zero-padded, so key order is the
 * chronological order the section list is already in.
 */
export function mergeSections<T>(
    sections: Section<T>[],
    extra: Bucket[],
    order: 'asc' | 'desc',
): Section<T>[] {
    const out = [...sections]
    const ahead = (a: string, b: string) => (order === 'desc' ? a > b : a < b)
    for (const b of extra) {
        if (out.some((s) => s.key === b.key)) continue
        const at = out.findIndex((s) => ahead(b.key, s.key))
        const section: Section<T> = {key: b.key, label: b.label, items: []}
        if (at === -1) out.push(section)
        else out.splice(at, 0, section)
    }
    return out
}
