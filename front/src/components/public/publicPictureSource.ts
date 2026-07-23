import {useMemo} from 'react'
import type {PictureDetail} from '@/lib/types'
import type {PictureSource} from '@/components/photos/pictureSource'
import type {PublicPictureDetail} from '@/api/publicShares'
import {downloadPublicOriginal, getPublicPictureDetail, getPublicPictureUrl,} from '@/api/publicShares'

/** Map a token-gated public detail into the `PictureDetail` the (read-only) lightbox reads. */
function toDetail(id: string, d: PublicPictureDetail): PictureDetail {
    return {
        id,
        filename: d.filename,
        mime_type: d.mime_type,
        file_size: d.file_size,
        width: d.width,
        height: d.height,
        captured_at: d.captured_at ?? null,
        ingested_at: d.ingested_at,
        updated_at: d.ingested_at,
        original_file_created_at: null,
        gps_lat: d.gps_lat ?? null,
        gps_lng: d.gps_lng ?? null,
        gps_alt: d.gps_alt ?? null,
        orientation: d.orientation,
        exif_data: (d.exif_data as Record<string, unknown>) ?? {},
        exif_sync_status: 'synced',
        owner_username: null,
        owner_instance_domain: null,
        creator: d.creator,
        creator_origin: d.creator,
        creator_value: null,
        creator_override: null,
        deleted_at: null,
        owner_deleted_at: null,
        owner_purge_at: null,
        local_exif_overrides: null,
        content_hash: null,
        copy_source_owner_username: null,
        copy_source_owner_instance: null,
        copy_source_picture_id: null,
        versions: [],
    }
}

/**
 * A read-only `PictureSource` backed by the token-gated public endpoints on the owner's backend, so the
 * shared `Lightbox`/carousel render a public share without any auth session. Keys are namespaced by
 * token so a public presign/detail never collides with an authenticated one in the shared query cache.
 */
export function usePublicPictureSource(args: {
    backendUrl: string
    token: string
    session: string | null
    canDownload: boolean
}): PictureSource {
    const {backendUrl, token, session, canDownload} = args
    return useMemo<PictureSource>(
        () => ({
            presign: async (id, v) => ({url: await getPublicPictureUrl(backendUrl, token, id, v, session)}),
            getDetail: async (id) => toDetail(id, await getPublicPictureDetail(backendUrl, token, id, session)),
            urlKey: (id, v) => ['publicUrl', token, id, v],
            detailKey: (id) => ['publicDetail', token, id],
            readOnly: true,
            canDownload,
            download: (id, filename) => downloadPublicOriginal(backendUrl, token, id, filename, session),
            // No onCloseFocus: the shared Lightbox default selects the viewed picture in the global store,
            // which is exactly what the public page now uses.
        }),
        [backendUrl, token, session, canDownload],
    )
}
