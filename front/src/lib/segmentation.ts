// Client-side helpers for the calendar-segmentation band model (feature 20).
// Pure functions mirroring the backend resolution (§7) for preview + lints; the
// backend remains authoritative — these are presentation/UX aids only.

import type {PartConfig, PartFormat, SegmentationBand, SegmentationConfig, SegmentationOffset, SegmentationPlaceholder,} from '@/lib/types'

export const PLACEHOLDERS: SegmentationPlaceholder[] = [
    'year',
    'iso_year',
    'quarter',
    'season',
    'month',
    'week',
    'day',
    'weekday',
    'daypart',
]

/** Placeholders that have a named form; the rest are numeric-only (§4.1). */
export const NAMED_PLACEHOLDERS = new Set<SegmentationPlaceholder>(['season', 'month', 'weekday', 'daypart'])

/** Default render is a name (rather than a number) for these (§4.1). */
const NAME_DEFAULT = new Set<SegmentationPlaceholder>(['season', 'weekday', 'daypart'])

/** Whether a placeholder renders as a name by default (vs a number) — mirrors §4.1. */
export function placeholderDefaultsToName(ph: SegmentationPlaceholder): boolean {
    return NAME_DEFAULT.has(ph)
}

export function emptyConfig(): SegmentationConfig {
    return {version: 1, root_tag: 'Photos', hemisphere: 'north', catch_all: null, bands: []}
}

export function newBand(): SegmentationBand {
    return {from: null, to: null, template: '{year}', enabled: true}
}

// ── Template parsing ────────────────────────────────────────────────────────

/** Placeholders referenced anywhere in a template, in order, deduped. */
export function templatePlaceholders(template: string): string[] {
    const out: string[] = []
    for (const m of template.matchAll(/\{([a-z_]+)\}/g)) {
        if (!out.includes(m[1])) out.push(m[1])
    }
    return out
}

function isKnownPlaceholder(name: string): name is SegmentationPlaceholder {
    return (PLACEHOLDERS as string[]).includes(name)
}

// ── Projection (mirror of the backend, §4.1 / §6) ───────────────────────────

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December']
const WEEKDAYS = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']
const SEASONS = ['Winter', 'Spring', 'Summer', 'Autumn'] // index by season number 1..4
const DAYPARTS = ['Night', 'Morning', 'Afternoon', 'Evening']

function applyOffset(d: Date, offset?: SegmentationOffset): Date {
    if (!offset) return d
    const out = new Date(d)
    if (offset.months) out.setMonth(out.getMonth() - offset.months)
    if (offset.days) out.setDate(out.getDate() - offset.days)
    if (offset.hours) out.setHours(out.getHours() - offset.hours)
    if (offset.minutes) out.setMinutes(out.getMinutes() - offset.minutes)
    return out
}

function isoWeek(d: Date): { week: number; year: number } {
    const date = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()))
    const dayNum = (date.getUTCDay() + 6) % 7
    date.setUTCDate(date.getUTCDate() - dayNum + 3)
    const firstThursday = new Date(Date.UTC(date.getUTCFullYear(), 0, 4))
    const week = 1 + Math.round(((date.getTime() - firstThursday.getTime()) / 86400000 - 3 + ((firstThursday.getUTCDay() + 6) % 7)) / 7)
    return {week, year: date.getUTCFullYear()}
}

function seasonIndex(month0: number, hemisphere: 'north' | 'south'): number {
    // month0 0..11; northern: Win=Dec-Feb(0), Spr=Mar-May(1), Sum=Jun-Aug(2), Aut=Sep-Nov(3)
    const northern = month0 === 11 || month0 <= 1 ? 0 : month0 <= 4 ? 1 : month0 <= 7 ? 2 : 3
    return hemisphere === 'south' ? (northern + 2) % 4 : northern
}

/** Raw numeric value + the default name (if any) for a placeholder at date `d`. */
function projectValue(name: SegmentationPlaceholder, d: Date, hemisphere: 'north' | 'south'): { num: number; name?: string } {
    switch (name) {
        case 'year':
            return {num: d.getFullYear()}
        case 'iso_year':
            return {num: isoWeek(d).year}
        case 'quarter':
            return {num: Math.floor(d.getMonth() / 3) + 1}
        case 'season': {
            const s = seasonIndex(d.getMonth(), hemisphere)
            return {num: s + 1, name: SEASONS[s]}
        }
        case 'month':
            return {num: d.getMonth() + 1, name: MONTHS[d.getMonth()]}
        case 'week':
            return {num: isoWeek(d).week}
        case 'day':
            return {num: d.getDate()}
        case 'weekday': {
            const wd = (d.getDay() + 6) % 7 // Mon=0
            return {num: wd + 1, name: WEEKDAYS[wd]}
        }
        case 'daypart': {
            const dp = Math.floor(d.getHours() / 6)
            return {num: dp + 1, name: DAYPARTS[dp]}
        }
    }
}

function strideStart(name: SegmentationPlaceholder, value: number, stride: number): number {
    if (stride <= 1) return value
    const origin = name === 'year' || name === 'iso_year' ? 0 : 1
    return origin + Math.floor((value - origin) / stride) * stride
}

function applyCase(s: string, c: PartFormat['case']): string {
    switch (c ?? 'pascal') {
        case 'lower':
            return s.toLowerCase()
        case 'upper':
            return s.toUpperCase()
        default:
            return s.charAt(0).toUpperCase() + s.slice(1).toLowerCase()
    }
}

function renderNum(n: number, fmt?: PartFormat): string {
    const pad = fmt?.pad ?? 0
    return pad > 0 ? String(n).padStart(pad, '0') : String(n)
}

/** Render a single placeholder for a date (mirror of §4.1/§5; preview-only). */
function renderPlaceholder(name: SegmentationPlaceholder, d: Date, part: PartConfig | undefined, hemisphere: 'north' | 'south'): string {
    const fmt = part?.format
    const stride = part?.stride ?? 1
    const {num, name: defaultName} = projectValue(name, d, hemisphere)

    const wantsNumeric = fmt?.numeric ?? !NAME_DEFAULT.has(name)

    const renderOne = (raw: number): string => {
        if (!wantsNumeric && defaultName) {
            const full = name === 'month' ? MONTHS[(raw - 1 + 12) % 12] : name === 'weekday' ? WEEKDAYS[(raw - 1 + 7) % 7] : name === 'season' ? SEASONS[(raw - 1 + 4) % 4] : name === 'daypart' ? DAYPARTS[(raw - 1 + 4) % 4] : defaultName
            const label = fmt?.abbrev ? full.slice(0, 3) : full
            return applyCase(label, fmt?.case)
        }
        return renderNum(raw, fmt)
    }

    if (stride <= 1) return renderOne(num)

    const start = strideStart(name, num, stride)
    const bound = fmt?.bound ?? 'start'
    if (bound === 'start') return renderOne(start)
    const inclusive = fmt?.inclusive_end ?? false
    const end = inclusive ? start + stride - 1 : start + stride
    if (bound === 'end') return renderOne(end)
    return `${renderOne(start)}${fmt?.range_sep ?? '_'}${renderOne(end)}`
}

/** Sanitize a rendered level to a valid ltree label (§4). */
function sanitizeLevel(s: string): string {
    return s.replace(/[^A-Za-z0-9_]+/g, '_').replace(/^_+|_+$/g, '')
}

/** Render a band's full path (relative to root) for `captured_at = d`. '' on failure. */
export function renderBandPath(band: SegmentationBand, d: Date, hemisphere: 'north' | 'south'): string {
    const shifted = applyOffset(d, band.offset)
    const levels = band.template.split('.')
    const out: string[] = []
    for (const level of levels) {
        const rendered = level.replace(/\{([a-z_]+)\}/g, (_, ph: string) => {
            if (!isKnownPlaceholder(ph)) return ''
            return renderPlaceholder(ph, shifted, band.parts?.[ph], hemisphere)
        })
        const clean = sanitizeLevel(rendered)
        if (clean) out.push(clean)
    }
    return out.join('.')
}

// ── Resolution + preview (§7) ───────────────────────────────────────────────

function dateInRange(d: Date, from: string | null, to: string | null): boolean {
    const t = d.getTime()
    if (from && t < parseDate(from).getTime()) return false
    if (to && t >= parseDate(to).getTime()) return false
    return true
}

export function parseDate(s: string): Date {
    const [y, m, day] = s.split('-').map(Number)
    return new Date(y, (m ?? 1) - 1, day ?? 1)
}

/** Index of the first enabled band covering `d`, or -1 (§7). */
export function resolveBandIndex(config: SegmentationConfig, d: Date): number {
    for (let i = 0; i < config.bands.length; i++) {
        const b = config.bands[i]
        if (b.enabled === false) continue
        if (dateInRange(d, b.from, b.to)) return i
    }
    return -1
}

export interface PreviewBucket {
    /** rendered path relative to root, e.g. "2024.August" */
    path: string
    /** band index that produced it */
    bandIndex: number
    /** representative sample date */
    sample: Date
}

/**
 * Walk a sample range monthly and collect the distinct buckets each band produces,
 * in chronological order. Used by the timeline preview.
 */
export function previewBuckets(config: SegmentationConfig, start: Date, end: Date, hemisphere: 'north' | 'south'): PreviewBucket[] {
    const seen = new Set<string>()
    const out: PreviewBucket[] = []
    const cursor = new Date(start.getFullYear(), start.getMonth(), 1)
    let guard = 0
    while (cursor <= end && guard++ < 2400) {
        const bi = resolveBandIndex(config, cursor)
        if (bi >= 0) {
            const path = renderBandPath(config.bands[bi], cursor, hemisphere)
            const key = `${bi}|${path}`
            if (path && !seen.has(key)) {
                seen.add(key)
                out.push({path, bandIndex: bi, sample: new Date(cursor)})
            }
        }
        cursor.setMonth(cursor.getMonth() + 1)
    }
    return out
}

// ── Parent/child display inference (§2 / §12) ───────────────────────────────

/** B's range ⊆ A's range (open ends are unbounded). */
function rangeContains(a: SegmentationBand, b: SegmentationBand): boolean {
    const aFrom = a.from ? parseDate(a.from).getTime() : -Infinity
    const aTo = a.to ? parseDate(a.to).getTime() : Infinity
    const bFrom = b.from ? parseDate(b.from).getTime() : -Infinity
    const bTo = b.to ? parseDate(b.to).getTime() : Infinity
    return aFrom <= bFrom && bTo <= aTo
}

/** `child`'s template extends `parent`'s (dot-segment prefix) — a subdivision (§2). */
function templatePrefixed(parent: string, child: string): boolean {
    const p = parent.split('.')
    const c = child.split('.')
    if (c.length < p.length) return false
    for (let i = 0; i < p.length; i++) {
        if (p[i] !== c[i]) return false
    }
    return c.length > p.length
}

/** `child` is more specific than `parent`: it subdivides its template or covers a strict sub-range. */
function refines(child: SegmentationBand, parent: SegmentationBand): boolean {
    return templatePrefixed(parent.template, child.template) || (rangeContains(parent, child) && !sameRange(parent, child))
}

/**
 * Display nesting depth per band index. An override/subdivision is *more specific* and therefore
 * *higher precedence*, so it sits **above** (lower index than) the broader band it refines; we show
 * it indented under that broader band — the nearest band **below** it whose range contains it and
 * which it refines. Presentation only — the data model is a flat ordered list (top = highest
 * precedence). Computed bottom-up so the parent's depth is known first.
 */
export function displayDepths(bands: SegmentationBand[]): number[] {
    const depths = new Array(bands.length).fill(0)
    for (let i = bands.length - 1; i >= 0; i--) {
        for (let j = i + 1; j < bands.length; j++) {
            if (rangeContains(bands[j], bands[i]) && refines(bands[i], bands[j])) {
                depths[i] = depths[j] + 1
                break
            }
        }
    }
    return depths
}

// ── Lints (§9, warn-only) ───────────────────────────────────────────────────

export interface Lint {
    bandIndex: number | null
    kind: 'overlap' | 'dead' | 'gap'
    message: string
}

function rangesOverlap(a: SegmentationBand, b: SegmentationBand): boolean {
    const aFrom = a.from ? parseDate(a.from).getTime() : -Infinity
    const aTo = a.to ? parseDate(a.to).getTime() : Infinity
    const bFrom = b.from ? parseDate(b.from).getTime() : -Infinity
    const bTo = b.to ? parseDate(b.to).getTime() : Infinity
    return aFrom < bTo && bFrom < aTo
}

export function lintConfig(config: SegmentationConfig): Lint[] {
    const lints: Lint[] = []
    const bands = config.bands

    // Dead band: fully shadowed by an equal-or-broader higher-precedence band (§9).
    for (let i = 0; i < bands.length; i++) {
        if (bands[i].enabled === false) continue
        for (let j = 0; j < i; j++) {
            if (bands[j].enabled === false) continue
            if (rangeContains(bands[j], bands[i]) || sameRange(bands[j], bands[i])) {
                lints.push({bandIndex: i, kind: 'dead', message: 'Shadowed by a higher band with an equal-or-broader range — it never fires.'})
                break
            }
        }
    }

    // Ambiguous overlap at same rank: overlapping ranges that aren't a clean
    // override (contained) / subdivision (template-prefixed) relationship (§9).
    for (let i = 0; i < bands.length; i++) {
        if (bands[i].enabled === false) continue
        for (let j = i + 1; j < bands.length; j++) {
            if (bands[j].enabled === false) continue
            if (!rangesOverlap(bands[i], bands[j])) continue
            const contained = rangeContains(bands[i], bands[j]) || rangeContains(bands[j], bands[i])
            const subdivision = templatePrefixed(bands[i].template, bands[j].template) || templatePrefixed(bands[j].template, bands[i].template)
            if (!contained && !subdivision) {
                lints.push({
                    bandIndex: j,
                    kind: 'overlap',
                    message: `Overlaps band #${i + 1} without being an override or subdivision — order decides ambiguously.`
                })
            }
        }
    }

    // Uncovered gap when no catch_all (§9).
    if (!config.catch_all && bands.some((b) => b.enabled !== false)) {
        if (hasGap(bands)) {
            lints.push({
                bandIndex: null,
                kind: 'gap',
                message: 'Timeline has uncovered gaps and there is no catch-all — those pictures get no segment tag.'
            })
        }
    }

    return lints
}

function sameRange(a: SegmentationBand, b: SegmentationBand): boolean {
    return a.from === b.from && a.to === b.to
}

function hasGap(bands: SegmentationBand[]): boolean {
    const active = bands.filter((b) => b.enabled !== false)
    if (active.length === 0) return true
    // Merge intervals; report a gap if the union isn't a single contiguous span.
    const ivals = active
        .map((b) => [b.from ? parseDate(b.from).getTime() : -Infinity, b.to ? parseDate(b.to).getTime() : Infinity] as const)
        .sort((a, b) => a[0] - b[0])
    let cursor = ivals[0][0]
    for (const [lo, hi] of ivals) {
        if (lo > cursor) return true
        cursor = Math.max(cursor, hi)
    }
    return false
}
