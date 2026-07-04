import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {batchEditTags, type BatchEditTagsBody, listAllTags, listPictureTags, renameTag} from '@/api/tags'
import {queryKeys} from '@/lib/constants'
import {invalidatePicturesAndTags} from '@/lib/invalidation'

export function useAllTags() {
    return useQuery({
        queryKey: queryKeys.tags(),
        queryFn: listAllTags,
    })
}

export function usePictureTags(pictureId: string | null) {
    return useQuery({
        queryKey: queryKeys.pictureTags(pictureId ?? ''),
        enabled: !!pictureId,
        queryFn: () => listPictureTags(pictureId!),
    })
}

export function useBatchEditTags() {
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: (body: BatchEditTagsBody) => batchEditTags(body),
        onSuccess: () => invalidatePicturesAndTags(queryClient),
    })
}

/**
 * Rename a tag subtree (edge case §7). The cascade also rewrites shares, tagging services and
 * hierarchies, so their caches are invalidated alongside pictures + tags.
 */
export function useRenameTag() {
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: ({oldTag, newTag}: { oldTag: string; newTag: string }) => renameTag(oldTag, newTag),
        onSuccess: () => {
            invalidatePicturesAndTags(queryClient)
            void queryClient.invalidateQueries({queryKey: ['tagging']})
            void queryClient.invalidateQueries({queryKey: ['hierarchies']})
            void queryClient.invalidateQueries({queryKey: ['shares']})
        },
    })
}
