import {useState} from 'react'
import {useMutation, useQueryClient} from '@tanstack/react-query'
import {toast} from 'sonner'
import {editPicture, editReceivedExif, getJob, restorePicture, trashPicture} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {invalidatePictures, invalidatePicturesAndTags, invalidateTags} from '@/lib/invalidation'
import type {EditPictureResponse, ExifEditMode, ExifField, ExifOverrides} from '@/lib/types'

const POLL_DELAYS_MS = [1000, 2000, 4000, 8000, 15000]

async function pollUntilDone(jobId: string): Promise<void> {
    for (const delay of POLL_DELAYS_MS) {
        await new Promise<void>((resolve) => setTimeout(resolve, delay))
        const job = await getJob(jobId)
        if (job.status === 'completed') return
        if (job.status === 'failed') {
            throw new Error(job.error_message ?? 'EXIF reconciliation failed')
        }
        // still pending/processing — continue loop
    }
    // Timed out (~30 s total), give up silently; file may reconcile later
}

export function useEditExif(pictureId: string) {
    const queryClient = useQueryClient()
    const [syncing, setSyncing] = useState(false)

    const mutation = useMutation({
        mutationFn: (body: { set?: Partial<ExifOverrides>; clear?: ExifField[] }) =>
            editPicture(pictureId, body),

        onSuccess: async (res: EditPictureResponse) => {
            // A metadata (EXIF) edit is a pipeline event: rules/segments can (re)assign tag
            invalidatePicturesAndTags(queryClient)

            if (res.exif_sync_status === 'pending' && res.job_id) {
                setSyncing(true)
                try {
                    await pollUntilDone(res.job_id)
                    // File is now reconciled: refresh pictures again.
                    invalidatePictures(queryClient)
                } catch (err) {
                    toast.error('EXIF file sync failed', {description: apiErrorMessage(err)})
                } finally {
                    setSyncing(false)
                }
            }
        },

        onError: (error: unknown) => {
            toast.error('Could not save EXIF', {description: apiErrorMessage(error)})
        },
    })

    return {mutation, syncing}
}

/** Body of a received-picture EXIF edit: a `set`/`clear` delta plus the edit mode. */
type ReceivedExifBody = { mode?: ExifEditMode; set?: Partial<ExifOverrides>; clear?: ExifField[] }

/**
 * EXIF edit for a **received** picture (feature 10). Supports both modes of
 * `POST /pictures/{id}/exif`:
 *
 * - `local` — a recipient-local override (DB-only, no file reconcile / job poll), returns `200`.
 * - `propose` — propose to the owner; lands asynchronously, returns `202`. On a `202` we surface a
 *   "sent to owner" toast since the authoritative value only arrives later via re-announce.
 *
 * Mirrors {@link useEditExif}'s `{mutation, syncing}` interface (`syncing` always false) so
 * {@link useExifDraft} can treat owned edits and received edits uniformly.
 */
export function useOverrideExif(pictureId: string) {
    const queryClient = useQueryClient()

    const mutation = useMutation({
        mutationFn: (body: ReceivedExifBody) => editReceivedExif(pictureId, body),
        onSuccess: (res) => {
            // A local override changes captured_at/GPS/… which can re-fire pipeline on the recipient's pictures
            invalidatePicturesAndTags(queryClient)
            // 202 Accepted: the proposal was accepted but the owner applies + re-announces
            // asynchronously, so the authoritative value lands a moment later.
            if (res.status === 202) {
                toast.success('Suggested to owner', {
                    description: 'The owner will apply it and it will sync back to everyone shortly.',
                })
            }
        },
        onError: (error: unknown) => {
            toast.error('Could not save EXIF', {description: apiErrorMessage(error)})
        },
    })

    return {mutation, syncing: false}
}

/** Soft-delete / restore mutations for pictures the user holds. */
export function useTrashMutations() {
    const queryClient = useQueryClient()
    const invalidate = () => {
        invalidatePictures(queryClient)
        invalidateTags(queryClient)
    }

    const trash = useMutation({
        mutationFn: trashPicture,
        onSuccess: invalidate,
        onError: (error: unknown) => toast.error('Could not move to trash', {description: apiErrorMessage(error)}),
    })

    const restore = useMutation({
        mutationFn: restorePicture,
        onSuccess: invalidate,
        onError: (error: unknown) => toast.error('Could not restore', {description: apiErrorMessage(error)}),
    })

    return {trash, restore}
}
