// GPS derivation math for the photos-fix tools (feature 30 §5). Time-weighted interpolation when
// exactly two anchors bracket a dated target; a plain centroid otherwise (never extrapolate past the
// ends). All pure — no network, no dependency.

export interface GpsAnchor {
    lat: number
    lng: number
    alt?: number | null
    /** NaiveDateTime "YYYY-MM-DDTHH:MM:SS" (or null). Used only for time-weighted interpolation. */
    time?: string | null
}

export type GpsMethod = 'interpolated' | 'centroid' | 'copy'

export interface GpsResult {
    lat: number
    lng: number
    alt: number | null
    method: GpsMethod
    /** Whether the two anchors sit far apart in time — interpolation is then a guess (§5.4). */
    farApart?: boolean
}

/** Epoch milliseconds for a naive datetime string, interpreted on a consistent local basis. */
export function naiveToMs(iso: string | null | undefined): number | null {
    if (!iso) return null
    const [datePart = '', timePart = '00:00:00'] = iso.split('T')
    const [y, mo, d] = datePart.split('-').map(Number)
    const [h = 0, mi = 0, s = 0] = timePart.split(':').map(Number)
    if (!y || !mo || !d) return null
    return new Date(y, mo - 1, d, h, mi, s).getTime()
}

const R = 6_371_000 // Earth radius (m)
const rad = (d: number) => (d * Math.PI) / 180

/** Great-circle distance in metres (haversine) — matches the backend `geo_near` ordering (§6). */
export function haversineM(lat1: number, lng1: number, lat2: number, lng2: number): number {
    const dLat = rad(lat2 - lat1)
    const dLng = rad(lng2 - lng1)
    const a =
        Math.sin(dLat / 2) ** 2 +
        Math.cos(rad(lat1)) * Math.cos(rad(lat2)) * Math.sin(dLng / 2) ** 2
    return 2 * R * Math.asin(Math.min(1, Math.sqrt(a)))
}

/** Format a coordinate pair with 2 decimals and N/S · E/W suffixes so it reads unambiguously as GPS. */
export function formatLatLng(lat: number, lng: number): string {
    const c = (v: number, pos: string, neg: string) => `${Math.abs(v).toFixed(2)}°${v >= 0 ? pos : neg}`
    return `${c(lat, 'N', 'S')}, ${c(lng, 'E', 'W')}`
}

/** Human distance label ("120 m" / "3.4 km"). */
export function formatDistance(m: number): string {
    if (m < 1000) return `${Math.round(m)} m`
    if (m < 10_000) return `${(m / 1000).toFixed(1)} km`
    return `${Math.round(m / 1000)} km`
}

const round6 = (n: number) => Math.round(n * 1e6) / 1e6

// Anchors more than this far apart in time make interpolation a guess (warn badge, §5.4).
const FAR_APART_MS = 6 * 60 * 60 * 1000 // 6 hours

/**
 * Time-weighted midpoint between two anchors for a target instant `t` that they bracket:
 * `p = p0 + (p1 − p0)·(t − t0)/(t1 − t0)` on lat/lng (and alt when both present).
 */
function interpolatePair(tMs: number, a: GpsAnchor, b: GpsAnchor): GpsResult {
    const t0 = naiveToMs(a.time)!
    const t1 = naiveToMs(b.time)!
    const [lo, hi] = t0 <= t1 ? [a, b] : [b, a]
    const loMs = Math.min(t0, t1)
    const hiMs = Math.max(t0, t1)
    const span = hiMs - loMs
    const f = span === 0 ? 0.5 : (tMs - loMs) / span
    const lat = lo.lat + (hi.lat - lo.lat) * f
    const lng = lo.lng + (hi.lng - lo.lng) * f
    let alt: number | null = null
    if (lo.alt != null && hi.alt != null) alt = Math.round(lo.alt + (hi.alt - lo.alt) * f)
    return {
        lat: round6(lat),
        lng: round6(lng),
        alt,
        method: 'interpolated',
        farApart: span > FAR_APART_MS,
    }
}

/** Plain average of the anchors' coordinates (no extrapolation). */
function centroid(refs: GpsAnchor[]): GpsResult {
    const n = refs.length
    const lat = refs.reduce((s, p) => s + p.lat, 0) / n
    const lng = refs.reduce((s, p) => s + p.lng, 0) / n
    const withAlt = refs.filter((p) => p.alt != null)
    const alt =
        withAlt.length === n && n > 0
            ? Math.round(withAlt.reduce((s, p) => s + (p.alt as number), 0) / n)
            : null
    return {lat: round6(lat), lng: round6(lng), alt, method: n === 1 ? 'copy' : 'centroid'}
}

/**
 * Derive a GPS point for a target from reference anchors (§5.4):
 *  - 1 reference → copy it (average of one = itself);
 *  - exactly 2 references that **bracket** a dated target in time → time-weighted interpolation;
 *  - otherwise (same-side pair, N > 2, or an undated target) → plain centroid.
 * Returns `null` when there are no usable references.
 */
export function deriveGps(targetTime: string | null | undefined, refs: GpsAnchor[]): GpsResult | null {
    const usable = refs.filter((r) => Number.isFinite(r.lat) && Number.isFinite(r.lng))
    if (usable.length === 0) return null
    if (usable.length === 1) return {...centroid(usable), method: 'copy'}

    const tMs = naiveToMs(targetTime)
    if (usable.length === 2 && tMs != null) {
        const [a, b] = usable
        const t0 = naiveToMs(a.time)
        const t1 = naiveToMs(b.time)
        if (t0 != null && t1 != null) {
            const lo = Math.min(t0, t1)
            const hi = Math.max(t0, t1)
            if (tMs >= lo && tMs <= hi) return interpolatePair(tMs, a, b)
        }
    }
    return centroid(usable)
}
