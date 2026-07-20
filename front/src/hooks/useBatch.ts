import {useMutation, useQueryClient} from '@tanstack/react-query'
import {toast} from 'sonner'
import {type BatchCreatorBody, batchEditExif, type BatchExifBody, batchRestore, batchSetCreator, batchTrash} from '@/api/pictures'
import {batchEditTags, type BatchEditTagsBody} from '@/api/tags'
import {apiErrorMessage} from '@/api/client'
import {invalidatePicturesAndTags, invalidateStorageDebounced} from '@/lib/invalidation'
import type {PictureSelection} from '@/lib/types'

/**
 * Batch write mutations over a selection (feature 14 §6). Each invalidates the picture/tag/browse
 * caches on success — which also refreshes the `/aggregate` panel (its key is under `['pictures']`).
 * Confirmation + dry-run are handled by {@link BatchConfirmDialog}; these are the apply calls.
 */
export function useBatchMutations() {
    const qc = useQueryClient()
    const invalidate = () => {
        invalidatePicturesAndTags(qc)
        void qc.invalidateQueries({queryKey: ['hierarchies']})
    }
    const onError = (label: string) => (e: unknown) => toast.error(label, {description: apiErrorMessage(e)})

    const trash = useMutation({
        mutationFn: (selection: PictureSelection) => batchTrash(selection),
        onSuccess: () => {
            invalidate()
            invalidateStorageDebounced(qc)
        },
        onError: onError('Could not move to trash'),
    })
    const restore = useMutation({
        mutationFn: (selection: PictureSelection) => batchRestore(selection),
        onSuccess: () => {
            invalidate()
            invalidateStorageDebounced(qc)
        },
        onError: onError('Could not restore'),
    })
    const tags = useMutation({
        mutationFn: (body: BatchEditTagsBody) => batchEditTags(body),
        onSuccess: invalidate,
        onError: onError('Could not update tags'),
    })
    const exif = useMutation({
        mutationFn: (body: BatchExifBody) => batchEditExif(body),
        onSuccess: invalidate,
        onError: onError('Could not edit EXIF'),
    })
    const creator = useMutation({
        mutationFn: (body: BatchCreatorBody) => batchSetCreator(body),
        onSuccess: invalidate,
        onError: onError('Could not set creator'),
    })

    return {trash, restore, tags, exif, creator}
}
