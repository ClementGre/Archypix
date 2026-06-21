import {apiClient} from './client'
import type {
    EditPictureResponse,
    ExifField,
    ExifOverrides,
    Job,
    OverrideExifResponse,
    PictureDetail,
    PictureListResponse,
    PictureVariant,
    TrashResponse,
} from '@/lib/types'

export interface UploadSlot {
    picture_id: string
    presigned_url: string
}

export interface CompleteUploadBody {
    mime_type?: string
    file_size?: number
    /** SHA-256 of the file (lowercase hex) — provisional ETag/dedupe key; the worker re-confirms it. */
    file_hash?: string
    width?: number
    height?: number
    captured_at?: string
    initial_tags?: string[]
    defer_pipeline?: boolean
}

export interface ListPicturesParams {
    page: number
    page_size: number
    sort?: string
    order?: string
    tag?: string
    owned_only?: boolean
    shared_with_me?: boolean
    include_deleted?: boolean
    captured_after?: string
    captured_before?: string
    thumbnail?: PictureVariant
}

export async function listPictures(params: ListPicturesParams): Promise<PictureListResponse> {
    // Build params object omitting undefined values so axios doesn't send empty keys.
    const query: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(params)) {
        if (v !== undefined) query[k] = v
    }
    const {data} = await apiClient.get<PictureListResponse>('/api/authenticated/pictures', {
        params: query,
    })
    return data
}

export async function getPicture(id: string): Promise<PictureDetail> {
    const {data} = await apiClient.get<PictureDetail>(`/api/authenticated/pictures/${id}`)
    return data
}

export async function getPictureUrl(
    id: string,
    variant: PictureVariant,
): Promise<{ url: string; variant: PictureVariant }> {
    const {data} = await apiClient.get<{ url: string; variant: PictureVariant }>(
        `/api/authenticated/pictures/${id}/url`,
        {params: {variant}},
    )
    return data
}

export async function editPicture(
    id: string,
    body: { set?: Partial<ExifOverrides>; clear?: ExifField[] },
): Promise<EditPictureResponse> {
    const {data} = await apiClient.post<EditPictureResponse>(
        `/api/authenticated/pictures/${id}/edit`,
        body,
    )
    return data
}

/**
 * Apply a recipient-local EXIF override to a received picture. DB-only — no file reconcile,
 * no job. `set` claims a sticky per-field override; `clear` drops the override so the owner's
 * value flows through again.
 */
export async function overrideExif(
    id: string,
    body: { set?: Partial<ExifOverrides>; clear?: ExifField[] },
): Promise<OverrideExifResponse> {
    const {data} = await apiClient.post<OverrideExifResponse>(
        `/api/authenticated/pictures/${id}/exif/override`,
        body,
    )
    return data
}

/** Soft-delete a picture the user holds (owned or received). */
export async function trashPicture(id: string): Promise<TrashResponse> {
    const {data} = await apiClient.post<TrashResponse>(`/api/authenticated/pictures/${id}/trash`)
    return data
}

/** Restore a soft-deleted picture (clears `deleted_at`). */
export async function restorePicture(id: string): Promise<TrashResponse> {
    const {data} = await apiClient.post<TrashResponse>(`/api/authenticated/pictures/${id}/restore`)
    return data
}

export async function getJob(id: string): Promise<Job> {
    const {data} = await apiClient.get<Job>(`/api/authenticated/jobs/${id}`)
    return data
}

export async function beginUploadBatch(filenames: string[]): Promise<UploadSlot[]> {
    const {data} = await apiClient.post<UploadSlot[]>('/api/authenticated/pictures/uploads/batch', {filenames})
    return data
}

export async function completeUpload(pictureId: string, body: CompleteUploadBody): Promise<{ id: string }> {
    const {data} = await apiClient.post<{ id: string }>(
        `/api/authenticated/pictures/uploads/${pictureId}/complete`,
        body,
    )
    return data
}

/**
 * Explicitly wake the tagging pipeline for the current user. Call once after a batch of uploads
 */
export async function wakePipeline(): Promise<void> {
    await apiClient.post('/api/authenticated/pictures/pipeline/wake')
}
