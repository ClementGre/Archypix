import type {QueryClient} from '@tanstack/react-query'
import {beginUploadBatch, completeUpload, restorePicture} from '@/api/pictures'
import {invalidatePictures, invalidatePicturesAndTags, invalidateStorageDebounced} from '@/lib/invalidation'

/** A presign slot, normalised across the authenticated and public upload backends. */
export interface UploadSlotResult {
    picture_id: string
    presigned_url: string | null
    /** The file already exists on the target library — no upload needed. */
    duplicate: boolean
    /** Auth only: the matched existing picture is in the trash (restorable). */
    was_deleted?: boolean
}

export interface UploadBeginFile {
    filename: string
    file_hash?: string
    size?: number
}

export interface UploadCtx {
    tags?: string[]
    label?: string
    /** Public contributions: the visitor's display name (creator credit). */
    contributor?: string
}

export interface UploadCompleteMeta {
    mime_type?: string
    file_size?: number
    file_hash?: string
}

/**
 * The upload backend + UI capabilities the shared `UploadDialog` reads, so the same dialog serves both
 * an authenticated library upload and an anonymous public-share contribution. Default is the auth source
 * (unchanged behaviour); the public page swaps in a token-gated one.
 */
export interface UploadSource {
    title: string
    begin: (files: UploadBeginFile[], ctx: UploadCtx) => Promise<UploadSlotResult[]>
    complete: (pictureId: string, meta: UploadCompleteMeta, ctx: UploadCtx) => Promise<void>
    /** Restore trashed dedup hits (auth only). */
    restore?: (pictureId: string) => Promise<void>
    onFirstSuccess?: (qc: QueryClient) => void
    onSettled?: (qc: QueryClient) => void
    // UI capabilities ──
    /** Require a contributor-name field before uploading (public contributions). */
    requireContributor: boolean
    /** Show the "apply tags to all" picker (auth). */
    showTagPicker: boolean
    /** Show the storage-quota preflight (auth). */
    showStoragePreflight: boolean
    /** Show the import-label summary + restore-from-trash affordance (auth). */
    showImportSummary: boolean
    /** Per-row message for a dedup hit. */
    dedupMessage: string
}

/** The authenticated library upload — the dialog's original behaviour. */
export const AUTH_UPLOAD_SOURCE: UploadSource = {
    title: 'Upload photos',
    begin: (files, ctx) => beginUploadBatch(files, ctx.tags?.length ? ctx.tags : undefined, ctx.label || undefined),
    complete: async (pictureId, meta, ctx) => {
        await completeUpload(pictureId, {
            mime_type: meta.mime_type,
            file_size: meta.file_size,
            file_hash: meta.file_hash,
            initial_tags: ctx.tags?.length ? ctx.tags : undefined,
            upload_label: ctx.label || undefined,
        })
    },
    restore: (pictureId) => restorePicture(pictureId).then(() => undefined),
    onFirstSuccess: (qc) => invalidatePictures(qc),
    onSettled: (qc) => {
        invalidatePicturesAndTags(qc)
        invalidateStorageDebounced(qc)
    },
    requireContributor: false,
    showTagPicker: true,
    showStoragePreflight: true,
    showImportSummary: true,
    dedupMessage: 'Already in your library',
}
