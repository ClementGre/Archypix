import {useQuery} from '@tanstack/react-query'
import {getPicture, listPictures} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {useGridItems} from '@/stores/gridItems'
import {deriveGps, type GpsAnchor, type GpsResult} from '@/lib/gpsInterpolation'
import {gridGpsNeighbours} from '@/lib/gridAnchors'
import type {PictureDetail} from '@/lib/types'

export interface FixAnchor extends GpsAnchor {
    id: string
    filename: string | null
    thumbnail_url: string | null
    orientation: number | null
}

export interface FixAnchors {
    before: FixAnchor | null
    after: FixAnchor | null
    /** Time-weighted midpoint of before/after when both present and they bracket the target. */
    proposed: GpsResult | null
    loading: boolean
    /** True when the target has no capture date — GPS interpolation needs a time anchor (§12.1). */
    undatedTarget: boolean
}

/** Interpret a naive datetime string as UTC for the `captured_before/after` bounds (feature 29 §5). */
const asUtc = (naive: string) => `${naive}Z`

function toAnchor(p: PictureDetail): FixAnchor {
    return {
        id: p.id,
        filename: p.filename,
        thumbnail_url: null,
        orientation: p.orientation,
        lat: p.gps_lat as number,
        lng: p.gps_lng as number,
        alt: p.gps_alt,
        time: p.captured_at,
    }
}

/**
 * Resolve the nearest GPS-bearing pictures before and after a target's capture instant (feature 30
 * §5.2–5.3): grid-local candidates first (already loaded, no fetch), falling back to the directed
 * bracketing lookup (feature 29 §5 — `captured_before/after` + `gps=present` + `page_size=1` per
 * side) when the grid has no neighbour on a side. Anchor coordinates are then fetched (the list item
 * never exposes raw GPS), and the time-weighted midpoint is computed client-side.
 */
export function useFixAnchors(target: PictureDetail | null): FixAnchors {
    const gridItems = useGridItems((s) => s.items)
    const capturedAt = target?.captured_at ?? null
    const enabled = !!target && !!capturedAt

    const local = enabled ? gridGpsNeighbours(gridItems, target!.id, capturedAt!) : {before: null, after: null}

    // Directed bracketing fallback — only when the grid has no neighbour on that side.
    const beforeLookup = useQuery({
        queryKey: ['fix-anchor', 'before', capturedAt],
        enabled: enabled && !local.before,
        staleTime: 60_000,
        queryFn: () =>
            listPictures({
                page: 1, page_size: 1, sort: 'captured_at', order: 'desc',
                captured_before: asUtc(capturedAt!), gps: 'present',
                thumbnail: 'small',
            }),
    })
    const afterLookup = useQuery({
        queryKey: ['fix-anchor', 'after', capturedAt],
        enabled: enabled && !local.after,
        staleTime: 60_000,
        queryFn: () =>
            listPictures({
                page: 1, page_size: 1, sort: 'captured_at', order: 'asc',
                captured_after: asUtc(capturedAt!), gps: 'present',
                thumbnail: 'small',
            }),
    })

    const beforeId = local.before?.id ?? beforeLookup.data?.items[0]?.id ?? null
    const afterId = local.after?.id ?? afterLookup.data?.items[0]?.id ?? null

    const beforeDetail = useQuery({
        queryKey: queryKeys.picture(beforeId ?? ''),
        enabled: !!beforeId,
        queryFn: () => getPicture(beforeId!),
    })
    const afterDetail = useQuery({
        queryKey: queryKeys.picture(afterId ?? ''),
        enabled: !!afterId,
        queryFn: () => getPicture(afterId!),
    })

    const gridThumb = (id: string | null) => gridItems.find((i) => i.id === id) ?? null

    const before: FixAnchor | null = beforeDetail.data
        ? {...toAnchor(beforeDetail.data), thumbnail_url: gridThumb(beforeId)?.thumbnail_url ?? beforeLookup.data?.items[0]?.thumbnail_url ?? null}
        : null
    const after: FixAnchor | null = afterDetail.data
        ? {...toAnchor(afterDetail.data), thumbnail_url: gridThumb(afterId)?.thumbnail_url ?? afterLookup.data?.items[0]?.thumbnail_url ?? null}
        : null

    const anchors = [before, after].filter((a): a is FixAnchor => !!a)
    const proposed = anchors.length ? deriveGps(capturedAt, anchors) : null

    const loading =
        beforeLookup.isFetching || afterLookup.isFetching || beforeDetail.isFetching || afterDetail.isFetching

    return {before, after, proposed, loading, undatedTarget: !!target && !capturedAt}
}
