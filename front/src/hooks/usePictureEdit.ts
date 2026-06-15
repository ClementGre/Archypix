import {useState} from 'react'
import {useMutation, useQueryClient} from '@tanstack/react-query'
import {toast} from 'sonner'
import {editPicture, getJob} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import type {EditPictureResponse, ExifField, ExifOverrides} from '@/lib/types'

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
            // Immediately reflect DB changes in the picture detail cache.
            void queryClient.invalidateQueries({queryKey: queryKeys.picture(pictureId)})
            void queryClient.invalidateQueries({queryKey: ['pictures']})

            if (res.exif_sync_status === 'pending' && res.job_id) {
                setSyncing(true)
                try {
                    await pollUntilDone(res.job_id)
                    // File is now reconciled — refresh picture detail again.
                    void queryClient.invalidateQueries({queryKey: queryKeys.picture(pictureId)})
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
