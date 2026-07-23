// Normalise a server timestamp (naive datetime, possibly with a trailing `Z` or fractional seconds)
// to the "YYYY-MM-DDTHH:MM:SS" NaiveDateTime string the EXIF write paths expect (no timezone).

/** Build "YYYY-MM-DDTHH:MM:SS" from epoch milliseconds on the local wall clock. */
export function msToNaive(ms: number): string {
    const d = new Date(ms)
    const p = (n: number, w = 2) => String(n).padStart(w, '0')
    return `${p(d.getFullYear(), 4)}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

export function toNaive(value: string | null | undefined): string | null {
    if (!value) return null
    // Drop any timezone marker and fractional seconds, keep date + HH:MM:SS.
    const m = value.match(/^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2}:\d{2})/)
    if (m) return `${m[1]}T${m[2]}`
    // Date only → midnight.
    const d = value.match(/^(\d{4}-\d{2}-\d{2})/)
    return d ? `${d[1]}T00:00:00` : null
}

/** Parse "YYYY-MM-DDTHH:MM:SS" (NaiveDateTime, no tz) into a local Date + "HH:MM" string. */
export function parseNaive(iso: string): { date: Date; time: string } {
    const [datePart = '', timePart = '00:00:00'] = iso.split('T')
    const [y = 2000, mo = 1, d = 1] = datePart.split('-').map(Number)
    return {date: new Date(y, mo - 1, d), time: timePart.slice(0, 5)}
}

/** Build "YYYY-MM-DDTHH:MM:SS" from a local Date + "HH:MM" string. */
export function buildNaive(date: Date, time: string): string {
    const y = date.getFullYear()
    const mo = String(date.getMonth() + 1).padStart(2, '0')
    const d = String(date.getDate()).padStart(2, '0')
    const t = time.length >= 5 ? time.slice(0, 5) + ':00' : '00:00:00'
    return `${y}-${mo}-${d}T${t}`
}
