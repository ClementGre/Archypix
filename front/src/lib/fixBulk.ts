// Compute per-target proposed values for the bulk fix preview (feature 30 §8), carrying **provenance**
// so the preview can show where each value came from, let the user switch a row's source, and filter
// by source. Date rows expose all available sources (filename / file date / upload); GPS rows carry
// the before/after anchors they were interpolated from.

import type {PictureDetail, PictureListItem} from '@/lib/types'
import type {FixValue} from '@/hooks/useFixApply'
import {dateSuggestions} from '@/lib/dateSuggestions'
import {deriveGps, type GpsAnchor} from '@/lib/gpsInterpolation'
import {gridGpsNeighbours} from '@/lib/gridAnchors'

export type DateProvenance = 'filename' | 'filename-alt' | 'source' | 'uploaded' | 'reference' | 'manual'
export type GpsProvenance = 'interpolated' | 'reference' | 'manual'
export type Provenance = DateProvenance | GpsProvenance

export const PROVENANCE_LABEL: Record<Provenance, string> = {
    filename: 'Filename',
    'filename-alt': 'Filename (swapped)',
    source: 'File date',
    uploaded: 'Upload date',
    reference: 'References',
    interpolated: 'Interpolated',
    manual: 'Edited',
}

/** One candidate date source for a row (lets the user switch a row's provenance). */
export interface DateSource {
    key: DateProvenance
    value: string
    lowConfidence?: boolean
}

/** A thumbnail reference (a GPS before/after anchor) shown in a bulk GPS row. */
export interface RowAnchor {
    thumbnail_url: string | null
    orientation: number | null
}

export interface BulkRow {
    id: string
    filename: string | null
    thumbnail_url: string | null
    orientation: number | null
    owned: boolean
    include: boolean
    value: FixValue | null
    provenance: Provenance | null
    /** Date only: the candidate sources the user can switch between. */
    dateSources?: DateSource[]
    /** GPS only: the before/after anchors the value was interpolated from. */
    before?: RowAnchor | null
    after?: RowAnchor | null
}

function base(it: PictureListItem): Pick<BulkRow, 'id' | 'filename' | 'thumbnail_url' | 'orientation' | 'owned' | 'include'> {
    return {id: it.id, filename: it.filename, thumbnail_url: it.thumbnail_url, orientation: it.orientation, owned: it.owned, include: true}
}

/** Per-target date rows with every available source; the best (filename → file date → upload) is picked. */
export function dateBulkRows(items: PictureListItem[]): BulkRow[] {
    return items.map((it) => {
        const sources: DateSource[] = dateSuggestions(it).map((s) => ({key: s.key, value: s.value, lowConfidence: s.lowConfidence}))
        const chosen = sources[0] ?? null
        return {
            ...base(it),
            value: chosen ? {captured_at: chosen.value} : null,
            provenance: chosen?.key ?? null,
            dateSources: sources,
        }
    })
}

const anchorView = (it: PictureListItem | null): RowAnchor | null =>
    it ? {thumbnail_url: it.thumbnail_url, orientation: it.orientation} : null

/**
 * Per-target GPS rows via grid-local interpolation. Anchor coordinates are fetched once per unique
 * anchor; each dated target's before/after feed the time-weighted midpoint (or centroid). Undated
 * targets and targets with no grid anchor yield a `null` value (skipped row). Each row keeps the
 * before/after anchor thumbnails so the preview can show what it derived from.
 */
export async function gpsBulkRows(
    items: PictureListItem[],
    grid: PictureListItem[],
    getDetail: (id: string) => Promise<PictureDetail>,
): Promise<BulkRow[]> {
    const anchorIds = new Set<string>()
    const perTarget = items.map((it) => {
        if (!it.captured_at) return {it, before: null as PictureListItem | null, after: null as PictureListItem | null}
        const {before, after} = gridGpsNeighbours(grid, it.id, it.captured_at)
        if (before) anchorIds.add(before.id)
        if (after) anchorIds.add(after.id)
        return {it, before, after}
    })

    const coords = new Map<string, GpsAnchor>()
    await Promise.all(
        [...anchorIds].map(async (id) => {
            try {
                const d = await getDetail(id)
                if (d.gps_lat != null && d.gps_lng != null) {
                    coords.set(id, {lat: d.gps_lat, lng: d.gps_lng, alt: d.gps_alt, time: d.captured_at})
                }
            } catch {
                // Unreachable anchor → that target falls back to whatever anchors resolved.
            }
        }),
    )

    return perTarget.map(({it, before, after}) => {
        const anchors = [before, after].map((a) => (a ? coords.get(a.id) : null)).filter((a): a is GpsAnchor => !!a)
        const g = anchors.length ? deriveGps(it.captured_at, anchors) : null
        return {
            ...base(it),
            value: g ? {gps_lat: g.lat, gps_lng: g.lng, gps_alt: g.alt} : null,
            provenance: g ? 'interpolated' : null,
            before: anchorView(before),
            after: anchorView(after),
        }
    })
}

/** Rows that all take one shared derived value (the reference average) — provenance `reference`. */
export function referenceBulkRows(items: PictureListItem[], value: FixValue | null): BulkRow[] {
    return items.map((it) => ({...base(it), value, provenance: value ? 'reference' : null}))
}
