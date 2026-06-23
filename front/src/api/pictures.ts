import {apiClient} from './client'
import type {
    AggregateRequest,
    AggregateResponse,
    BatchDryRun,
    BatchExifMode,
    BatchExifResult,
    EditPictureResponse,
    ExifEditMode,
    ExifField,
    ExifOverrides,
    Job,
    OverrideExifResponse,
    PictureDetail,
    PictureListResponse,
    PictureSelection,
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

/**
 * Download a picture's **original** file. Resolves the presigned URL, then fetches
 * the bytes and saves them under the picture's filename. Falls back to opening the
 * URL in a new tab if the cross-origin fetch is blocked (e.g. S3 CORS), since the
 * `download` attribute is ignored for cross-origin links.
 */
export async function downloadOriginal(id: string, filename?: string | null): Promise<void> {
    const {url} = await getPictureUrl(id, 'original')
    try {
        const res = await fetch(url)
        if (!res.ok) throw new Error(`HTTP ${res.status}`)
        const blob = await res.blob()
        const objectUrl = URL.createObjectURL(blob)
        const a = document.createElement('a')
        a.href = objectUrl
        a.download = filename || 'photo'
        document.body.appendChild(a)
        a.click()
        a.remove()
        URL.revokeObjectURL(objectUrl)
    } catch {
        window.open(url, '_blank', 'noopener')
    }
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

/** Result of {@link editReceivedExif}: the body plus the HTTP status (200 local / 202 propose). */
export interface ReceivedExifResult {
    data: OverrideExifResponse
    /** 200 for a local override, 202 when a proposal was accepted by the owner's backend. */
    status: number
}

/**
 * Edit a **received** picture's EXIF (`doc/features/10 §4.1`). Two modes:
 *
 * - `mode: 'local'` (default) — a recipient-local override. DB-only, no file reconcile, no job.
 *   `set` claims a sticky per-field override; `clear` drops it so the owner's value flows through
 *   again. Always permitted. Returns `200`.
 * - `mode: 'propose'` — propose the edit to the owner, who auto-applies + re-announces so all
 *   recipients converge. Requires the incoming share to grant editing (`403` otherwise). The
 *   proposed fields' local overrides are cleared. Lands asynchronously — returns `202`.
 */
export async function editReceivedExif(
    id: string,
    body: { mode?: ExifEditMode; set?: Partial<ExifOverrides>; clear?: ExifField[] },
): Promise<ReceivedExifResult> {
    const res = await apiClient.post<OverrideExifResponse>(`/api/authenticated/pictures/${id}/exif`, body)
    return {data: res.data, status: res.status}
}

// ── Batch operations (feature 14 §6.11) ──────────────────────────────────────

/**
 * Type-aware aggregation over a selection (§4). Server-side GROUP BY / conditional aggregate,
 * so a select-all of thousands of pictures is never materialised. `sections` keeps it cheap —
 * the sidebar fetches `summary` eagerly and `tags`/`exif` only when those sections open.
 */
export async function aggregatePictures(body: AggregateRequest): Promise<AggregateResponse> {
    const {data} = await apiClient.post<AggregateResponse>('/api/authenticated/pictures/aggregate', body)
    return data
}

/** Body shared by the batch EXIF edit endpoint (`PATCH /pictures/exif`). */
export interface BatchExifBody {
    selection: PictureSelection
    set?: Partial<ExifOverrides>
    clear?: ExifField[]
    mode?: BatchExifMode
    dry_run?: boolean
}

/** Batch EXIF edit over a selection (§5–§6). With `dry_run` returns the affected breakdown. */
export async function batchEditExif(body: BatchExifBody & { dry_run: true }): Promise<BatchDryRun>
export async function batchEditExif(body: BatchExifBody): Promise<BatchExifResult>
export async function batchEditExif(body: BatchExifBody): Promise<BatchExifResult | BatchDryRun> {
    const {data} = await apiClient.patch<BatchExifResult | BatchDryRun>('/api/authenticated/pictures/exif', body)
    return data
}

/** Batch soft-delete over a selection (§6). With `dry_run` returns `{ affected }` only. */
export async function batchTrash(selection: PictureSelection, dryRun = false): Promise<BatchDryRun> {
    const {data} = await apiClient.post<BatchDryRun>('/api/authenticated/pictures/trash', {selection, dry_run: dryRun})
    return data
}

/** Batch restore over a selection (§6). The selection must include the trashed rows. */
export async function batchRestore(selection: PictureSelection, dryRun = false): Promise<BatchDryRun> {
    const {data} = await apiClient.post<BatchDryRun>('/api/authenticated/pictures/restore', {selection, dry_run: dryRun})
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
