// Filename → capture-date guesser (feature 30 §6.1). A shared, tested parser that tries as many
// forms as possible and returns a best guess with the matched pattern + a confidence flag. Offering
// an editable guess beats suggesting nothing, so it errs toward *a* result; the user always makes
// the final call.
//
// Rules (§6.1):
//  - Validity is a tiebreak, not a veto: an invalid reading (month 13, day 32, year out of range) is
//    dropped so the next interpretation can win rather than suppressing the suggestion entirely.
//  - Ambiguity (both components ≤ 12) is resolved by any component > 12, else a default order (DMY),
//    marked low-confidence, and the swapped reading is offered as `alternative`.
//  - Confidence is surfaced, never hidden: epoch and ambiguous matches are still offered, tagged low.

export type DateConfidence = 'high' | 'low'

export interface FilenameDateGuess {
    /** NaiveDateTime "YYYY-MM-DDTHH:MM:SS" (no timezone, local wall clock — see §6/§10). */
    value: string
    /** Human label of the pattern that matched, shown on the chip. */
    pattern: string
    confidence: DateConfidence
    /** Swapped-order reading for an ambiguous DD/MM vs MM/DD match (§6.1), offered as a 2nd chip. */
    alternative?: FilenameDateGuess
}

const NOW_YEAR = new Date().getFullYear()

const MONTHS: Record<string, number> = {
    jan: 1, feb: 2, mar: 3, apr: 4, may: 5, jun: 6,
    jul: 7, aug: 8, sep: 9, oct: 10, nov: 11, dec: 12,
}

/** Validate y/mo/d(+time) as a real calendar date and build the NaiveDateTime, or `null` if invalid. */
function build(
    y: number, mo: number, d: number,
    h = 0, mi = 0, s = 0,
): string | null {
    if (y < 1900 || y > NOW_YEAR + 1) return null
    if (mo < 1 || mo > 12) return null
    if (d < 1 || d > 31) return null
    if (h > 23 || mi > 59 || s > 59) return null
    // Reject impossible day-of-month (round-trips through Date to catch Feb 30 etc.).
    const probe = new Date(y, mo - 1, d)
    if (probe.getFullYear() !== y || probe.getMonth() !== mo - 1 || probe.getDate() !== d) return null
    const p = (n: number, w = 2) => String(n).padStart(w, '0')
    return `${p(y, 4)}-${p(mo)}-${p(d)}T${p(h)}:${p(mi)}:${p(s)}`
}

/** Strip the trailing file extension (if any) before scanning. */
function stripExt(name: string): string {
    return name.replace(/\.[A-Za-z0-9]{1,5}$/, '')
}

type Matcher = (s: string) => FilenameDateGuess | null

// Ordered roughly by confidence; the first matcher that yields a *valid* date wins.
const MATCHERS: Matcher[] = [
    // 1. YYYYMMDD[sep]HHMMSS — the dominant camera/phone form (IMG_20230815_143000, PXL_..., 20230815-143000).
    (s) => {
        const m = s.match(/(19|20)\d{2}(\d{2})(\d{2})[ _\-T.]?(\d{2})(\d{2})(\d{2})/)
        if (!m) return null
        const full = m[0]
        const yy = +full.slice(0, 4), mo = +full.slice(4, 6), d = +full.slice(6, 8)
        const v = build(yy, mo, d, +m[4], +m[5], +m[6])
        return v ? {value: v, pattern: 'Date & time in name', confidence: 'high'} : null
    },
    // 2. YYYY[sep]MM[sep]DD[sep]HH[sep]MM[sep]SS — separated date + time (2023-08-15 14.30.00 / T14:30:00).
    (s) => {
        const m = s.match(/(19|20)\d{2}[-_.](\d{1,2})[-_.](\d{1,2})[ _T]+(\d{1,2})[-_.:h](\d{2})(?:[-_.:m](\d{2}))?/)
        if (!m) return null
        const y = +m[0].slice(0, 4)
        const v = build(y, +m[2], +m[3], +m[4], +m[5], m[6] ? +m[6] : 0)
        return v ? {value: v, pattern: 'Date & time in name', confidence: 'high'} : null
    },
    // 3. WhatsApp: IMG-YYYYMMDD-WAxxxx (date only).
    (s) => {
        const m = s.match(/\b(?:IMG|VID)-((19|20)\d{2})(\d{2})(\d{2})-WA\d+/i)
        if (!m) return null
        const v = build(+m[1], +m[3], +m[4])
        return v ? {value: v, pattern: 'WhatsApp date', confidence: 'high'} : null
    },
    // 4. YYYY[sep]MM[sep]DD (date only) — hyphen/dot/underscore separated.
    (s) => {
        const m = s.match(/(19|20)\d{2}[-_.](\d{1,2})[-_.](\d{1,2})/)
        if (!m) return null
        const v = build(+m[0].slice(0, 4), +m[2], +m[3])
        return v ? {value: v, pattern: 'Date in name', confidence: 'high'} : null
    },
    // 5. Bare YYYYMMDD (8 contiguous digits, date only) — e.g. Screenshot_20230815.
    (s) => {
        const m = s.match(/(?<!\d)((19|20)\d{2})(\d{2})(\d{2})(?!\d)/)
        if (!m) return null
        const v = build(+m[1], +m[3], +m[4])
        return v ? {value: v, pattern: 'Date in name', confidence: 'high'} : null
    },
    // 6. Textual month: "15 Aug 2023", "15-Aug-2023", "Aug 15 2023", "Aug 15, 2023".
    (s) => {
        const lower = s.toLowerCase()
        let m = lower.match(/(?<!\d)(\d{1,2})[ _\-.]?(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*[ _\-.,]*(\d{4})/)
        if (m) {
            const v = build(+m[3], MONTHS[m[2]], +m[1])
            if (v) return {value: v, pattern: 'Written date', confidence: 'high'}
        }
        m = lower.match(/(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*[ _\-.]+(\d{1,2})[ _\-.,]+(\d{4})/)
        if (m) {
            const v = build(+m[3], MONTHS[m[1]], +m[2])
            if (v) return {value: v, pattern: 'Written date', confidence: 'high'}
        }
        return null
    },
    // 7. Ambiguous DD[sep]MM[sep]YYYY / MM[sep]DD[sep]YYYY (both ≤ 12 unless a component disambiguates).
    (s) => {
        const m = s.match(/(?<!\d)(\d{1,2})[-_./](\d{1,2})[-_./]((?:19|20)\d{2})(?!\d)/)
        if (!m) return null
        const a = +m[1], b = +m[2], y = +m[3]
        // Disambiguate by any component > 12; else default day-first (DMY), low-confidence.
        let primary: string | null
        let secondary: string | null
        let ambiguous = false
        if (a > 12 && b <= 12) {
            primary = build(y, b, a) // a=day, b=month
            secondary = null
        } else if (b > 12 && a <= 12) {
            primary = build(y, a, b) // a=month, b=day
            secondary = null
        } else {
            ambiguous = true
            primary = build(y, b, a) // default DMY
            secondary = build(y, a, b) // swapped MDY
        }
        if (!primary) {
            // Primary reading invalid → fall back to the other so we still offer *a* result (§6.1).
            if (secondary) return {value: secondary, pattern: 'Date in name (order guessed)', confidence: 'low'}
            return null
        }
        const guess: FilenameDateGuess = {
            value: primary,
            pattern: ambiguous ? 'Date in name (order guessed)' : 'Date in name',
            confidence: ambiguous ? 'low' : 'high',
        }
        if (ambiguous && secondary && secondary !== primary) {
            guess.alternative = {value: secondary, pattern: 'Swapped day/month', confidence: 'low'}
        }
        return guess
    },
    // 8. Unix epoch (13-digit ms or 10-digit s) — low-confidence, easily a coincidental number.
    (s) => {
        const m = s.match(/(?<!\d)(\d{13}|\d{10})(?!\d)/)
        if (!m) return null
        const raw = m[1]
        const ms = raw.length === 13 ? +raw : +raw * 1000
        const d = new Date(ms)
        if (isNaN(d.getTime())) return null
        const v = build(d.getFullYear(), d.getMonth() + 1, d.getDate(), d.getHours(), d.getMinutes(), d.getSeconds())
        return v ? {value: v, pattern: 'Unix timestamp', confidence: 'low'} : null
    },
]

/**
 * Best-effort capture date parsed from a filename. Returns `null` only when nothing plausible is
 * found; otherwise the first matcher (confidence order) yielding a real calendar date, with the
 * matched pattern label and a confidence flag (and, for ambiguous day/month, a swapped alternative).
 */
export function parseFilenameDate(filename: string | null | undefined): FilenameDateGuess | null {
    if (!filename) return null
    const s = stripExt(filename)
    for (const match of MATCHERS) {
        const g = match(s)
        if (g) return g
    }
    return null
}
