import {keepPreviousData, useInfiniteQuery} from '@tanstack/react-query'
import {listPictures} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import type {PictureFilters, PictureVariant} from '@/lib/types'

export function usePictures(
    filters: PictureFilters,
    opts?: {
        enabled?: boolean
        variant?: PictureVariant
        /**
         * A reference point for per-row great-circle distances (`distance_m` on each item), independent
         * of the sort — the photos-fix date mode passes the picture-being-fixed's GPS so the grid badges
         * "distance from it" without reordering (feature 30 §3). A `geo_near` sort's own `near_*`
         * reference (from `filters`) takes precedence when set.
         */
        geoRef?: { lat: number; lng: number } | null
    },
) {
    const variant = opts?.variant ?? 'medium'
    const geoRef = opts?.geoRef ?? null
    // A geo_near sort carries its own reference; otherwise fall back to the fix-mode geoRef.
    const nearLat = filters.sort === 'geo_near' ? filters.nearLat : geoRef?.lat ?? null
    const nearLng = filters.sort === 'geo_near' ? filters.nearLng : geoRef?.lng ?? null
    return useInfiniteQuery({
        queryKey: [...queryKeys.pictures(filters), variant, nearLat, nearLng],
        enabled: opts?.enabled ?? true,
        // Keep the current grid visible while a zoom-change (variant) or filter refetch lands.
        placeholderData: keepPreviousData,
        initialPageParam: 1,
        queryFn: ({pageParam}) => {
            // `tag` is the primary include; merge any extra sidebar includes into include_tags.
            const include = [...(filters.tag ? [filters.tag] : []), ...(filters.include ?? [])]
            const params = {
                page: pageParam as number,
                page_size: 50,
                thumbnail: variant,
                ...(filters.sort ? {sort: filters.sort} : {sort: 'ingested_at' as const}),
                ...(filters.order ? {order: filters.order} : {order: 'desc' as const}),
                ...(filters.scope === 'owned' ? {owned_only: true} : {}),
                ...(filters.scope === 'shared' ? {shared_with_me: true} : {}),
                ...(include.length ? {include_tags: include.join(','), match: 'all' as const} : {}),
                ...(filters.exclude?.length ? {exclude_tags: filters.exclude.join(',')} : {}),
                ...(filters.exact?.length ? {exact: filters.exact.join(',')} : {}),
                ...(filters.trash && filters.trash !== 'exclude' ? {trash: filters.trash} : {}),
                ...(filters.capturedAfter ? {captured_after: filters.capturedAfter} : {}),
                ...(filters.capturedBefore ? {captured_before: filters.capturedBefore} : {}),
                ...(filters.missingAny
                    ? {missing_any: true}
                    : {
                        ...(filters.gps && filters.gps !== 'any' ? {gps: filters.gps} : {}),
                        ...(filters.captureDate && filters.captureDate !== 'any'
                            ? {capture_date: filters.captureDate}
                            : {}),
                    }),
                ...(filters.sort === 'time_near' && filters.nearTime ? {near_time: filters.nearTime} : {}),
                ...(nearLat != null && nearLng != null ? {near_lat: nearLat, near_lng: nearLng} : {}),
                ...(filters.undatedFirst ? {undated_first: true} : {}),
            }
            return listPictures(params)
        },
        getNextPageParam: (last) =>
            last.page * last.page_size < last.total ? last.page + 1 : undefined,
    })
}
