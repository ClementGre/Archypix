import {createContext, useContext} from 'react'
import type {PictureDetail, PictureVariant} from '@/lib/types'
import {downloadOriginal, getPicture, getPictureUrl} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'

/**
 * Data access + capabilities the `Lightbox` (and its carousel/image children) read, so the same
 * viewer renders against either the authenticated backend or a token-gated public share. The default
 * context value is the authenticated behaviour, so the main gallery keeps its exact query keys and
 * cache reuse — only the public page swaps in a different source.
 */
export interface PictureSource {
    /** Presign a variant for a picture (`url` is null when the variant has no object). */
    presign: (id: string, variant: PictureVariant) => Promise<{ url: string | null }>
    /** Fetch a picture's detail (mime/size + rotate seed). */
    getDetail: (id: string) => Promise<PictureDetail>
    /** Query key for a presign — namespaced per source so auth and public caches never collide. */
    urlKey: (id: string, variant: PictureVariant) => readonly unknown[]
    /** Query key for a detail fetch. */
    detailKey: (id: string) => readonly unknown[]
    /** Read-only viewer: hides trash / copy / rotate / EXIF-edit affordances (public view). */
    readOnly: boolean
    /** Whether the original file may be fetched (download, original-quality, media playback). */
    canDownload: boolean
    /** Download the original under its filename. */
    download: (id: string, filename: string | null) => Promise<void>
    /** What to focus when the lightbox closes on a picture (auth: select it; public: local select). */
    onCloseFocus?: (id: string) => void
}

/** The authenticated source — preserves the gallery's existing keys and full read/write capabilities. */
export const AUTH_PICTURE_SOURCE: PictureSource = {
    presign: (id, v) => getPictureUrl(id, v),
    getDetail: (id) => getPicture(id),
    urlKey: (id, v) => ['pictures', 'url', id, v],
    detailKey: (id) => queryKeys.picture(id),
    readOnly: false,
    canDownload: true,
    download: (id, filename) => downloadOriginal(id, filename),
}

const PictureSourceContext = createContext<PictureSource>(AUTH_PICTURE_SOURCE)
export const PictureSourceProvider = PictureSourceContext.Provider
export const usePictureSource = () => useContext(PictureSourceContext)
