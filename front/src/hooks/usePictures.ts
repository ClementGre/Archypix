import {useInfiniteQuery} from '@tanstack/react-query'
import {listPictures} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import type {PictureFilters} from '@/lib/types'

export function usePictures(filters: PictureFilters) {
    return useInfiniteQuery({
        queryKey: queryKeys.pictures(filters),
        initialPageParam: 1,
        queryFn: ({pageParam}) => {
            const params = {
                page: pageParam as number,
                page_size: 50,
                thumbnail: 'medium' as const,
                ...(filters.sort ? {sort: filters.sort} : {sort: 'ingested_at' as const}),
                ...(filters.order ? {order: filters.order} : {order: 'desc' as const}),
                ...(filters.scope === 'owned' ? {owned_only: true} : {}),
                ...(filters.scope === 'shared' ? {shared_with_me: true} : {}),
                ...(filters.tag ? {tag: filters.tag} : {}),
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
