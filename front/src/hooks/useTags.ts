import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {batchEditTags, listAllTags, listPictureTags} from '@/api/tags'
import {queryKeys} from '@/lib/constants'

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
        mutationFn: batchEditTags,
        onSuccess: () => {
            void queryClient.invalidateQueries({queryKey: ['pictures']})
            void queryClient.invalidateQueries({queryKey: ['tags']})
        },
    })
}
