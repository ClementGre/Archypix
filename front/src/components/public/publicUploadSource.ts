import {useMemo} from 'react'
import type {UploadSource} from '@/components/photos/uploadSource'
import {publicCompleteUpload, publicUploadBatch} from '@/api/publicShares'
import {usePublicShare} from '@/components/public/context'

/**
 * A token-gated `UploadSource` for anonymous public-share contributions: a contributor name is required
 * and stamped as the creator, the album tag is forced server-side (no tag picker), and there is no
 * storage/import/restore surface. A dedup hit is reported as the owner already having the picture.
 */
export function usePublicUploadSource(): UploadSource {
    const {backendUrl, token} = usePublicShare()
    return useMemo<UploadSource>(
        () => ({
            title: 'Upload to this public share',
            begin: async (files, ctx) => {
                const slots = await publicUploadBatch(backendUrl, token, ctx.contributor ?? '', files)
                return slots.map((s) => ({
                    picture_id: s.picture_id,
                    presigned_url: s.presigned_url,
                    duplicate: s.rejected,
                }))
            },
            complete: async (pictureId, meta, ctx) => {
                await publicCompleteUpload(backendUrl, token, pictureId, {
                    contributor_name: ctx.contributor ?? '',
                    mime_type: meta.mime_type,
                    file_size: meta.file_size,
                    file_hash: meta.file_hash,
                })
            },
            onFirstSuccess: (qc) => void qc.invalidateQueries({queryKey: ['publicPictures']}),
            onSettled: (qc) => void qc.invalidateQueries({queryKey: ['publicPictures']}),
            requireContributor: true,
            showTagPicker: false,
            showStoragePreflight: false,
            showImportSummary: false,
            dedupMessage: 'The owner already has this picture',
        }),
        [backendUrl, token],
    )
}
