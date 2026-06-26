import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {batchEditTags, type BatchEditTagsBody, listAllTags, listPictureTags} from '@/api/tags'
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
