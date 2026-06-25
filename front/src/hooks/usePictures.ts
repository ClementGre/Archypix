import {keepPreviousData, useInfiniteQuery} from '@tanstack/react-query'
import {listPictures} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import type {PictureFilters, PictureVariant} from '@/lib/types'

export function usePictures(
    filters: PictureFilters,
    opts?: { enabled?: boolean; variant?: PictureVariant },
) {
    const variant = opts?.variant ?? 'medium'
    return useInfiniteQuery({
        queryKey: [...queryKeys.pictures(filters), variant],
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
                ...(filters.includeDeleted ? {include_deleted: true} : {}),
                ...(filters.capturedAfter ? {captured_after: filters.capturedAfter} : {}),
                ...(filters.capturedBefore ? {captured_before: filters.capturedBefore} : {}),
            }
            return listPictures(params)
        },
        getNextPageParam: (last) =>
            last.page * last.page_size < last.total ? last.page + 1 : undefined,
    })
}
