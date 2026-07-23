import {apiClient} from './client'
import type {
    AggregateRequest,
    AggregateResponse,
    BatchDryRun,
    BatchExifMode,
    BatchExifResult,
    CreatorEditMode,
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
    PresenceFilter,
    SetCreatorResponse,
    TrashFilter,
    TrashResponse,
} from '@/lib/types'

export interface UploadSlot {
    picture_id: string
    /** Present for a fresh file (PUT the bytes here); `null` for a deduplicated picture. */
    presigned_url: string | null
    /** True when the file's hash already matched an existing owned picture — no upload needed. */
    duplicate: boolean
    /** True when the matched existing picture is in the trash (it is NOT auto-restored). */
    was_deleted: boolean
}

/** One file requested in a batch presign: its name, (for dedup) its SHA-256 lowercase hex, and
 *  (for the storage-quota reservation, feature 22) its byte size. */
export interface BatchUploadFileInput {
    filename: string
    file_hash?: string
    size?: number
}

export interface CompleteUploadBody {
    mime_type?: string
    file_size?: number
    /** SHA-256 of the file (lowercase hex) — provisional ETag/dedupe key; the worker re-confirms it. */
    file_hash?: string
    width?: number
    height?: number
    captured_at?: string
    /** Optional source file creation time (feature 30 §10). The web upload leaves it unset (browsers
     *  expose no creation date); populated via WebDAV's `X-OC-CTime` instead. */
    original_file_created_at?: string
    initial_tags?: string[]
    /** Front-fixed import label (`Uploaded_YYYY_MM_DD_HH_MM`) — tags the new picture (feature 15). */
    upload_label?: string
    defer_pipeline?: boolean
}

export interface ListPicturesParams {
    page: number
    page_size: number
    sort?: string
    order?: string
    /** Comma-separated ltree paths (inclusive), combined per `match`. */
    include_tags?: string
    exclude_tags?: string
    /** Comma-separated ltree paths matched exactly (strict tag navigation). */
    exact?: string
    match?: 'all' | 'any'
    untagged?: boolean
    owned_only?: boolean
    shared_with_me?: boolean
    /** Trash-membership state: `exclude` (default) | `include` | `only`. Omitted when `exclude`. */
    trash?: TrashFilter
    captured_after?: string
    captured_before?: string
    /** Presence filters (feature 29 §4): `any` (omit) | `present` | `missing`. */
    gps?: PresenceFilter
    capture_date?: PresenceFilter
    /** OR convenience; 400 if combined with a non-`any` `gps`/`capture_date`. */
    missing_any?: boolean
    /** Proximity-sort references (feature 29 §6). */
    near_time?: string
    near_lat?: number
    near_lng?: number
    /** Date-fix mode (feature 30 §4): float undated pictures to the top of the current sort. */
    undated_first?: boolean
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
): Promise<{ url: string | null; variant: PictureVariant }> {
    // `url` is null when the requested variant has no object — a thumbnail variant on a picture with
    // no generated thumbnail (pending, or a non-thumbnailable format). The `original` always exists.
    const {data} = await apiClient.get<{ url: string | null; variant: PictureVariant }>(
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
    if (!url) throw new Error('No download URL available')
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
 *   `set` claims a sticky per-field override; `empty` claims it as empty/`null` (shadows a present
 *   owner value with emptiness); `clear` drops the claim so the owner's value flows through again.
 *   Always permitted. Returns `200`.
 * - `mode: 'propose'` — propose the edit to the owner, who auto-applies + re-announces so all
 *   recipients converge. Requires the incoming share to grant editing (`403` otherwise). The
 *   proposed fields' local overrides are cleared; `empty` folds into `clear`. Lands asynchronously —
 *   returns `202`.
 */
export async function editReceivedExif(
    id: string,
    body: { mode?: ExifEditMode; set?: Partial<ExifOverrides>; empty?: ExifField[]; clear?: ExifField[] },
): Promise<ReceivedExifResult> {
    const res = await apiClient.post<OverrideExifResponse>(`/api/authenticated/pictures/${id}/exif`, body)
    return {data: res.data, status: res.status}
}

/**
 * Set a picture's **creator** attribution credit (feature 26 §7). For an **owned** picture this sets
 * the owner-authoritative `creator` (`mode` ignored; `value` null/blank ⇒ reset to owner default);
 * for a **received** picture `mode: 'local'` (default) sets the recipient-local override (null/blank
 * clears it). A manual `value` beginning with `@`/`#` is rejected server-side (`400`) — the sigil
 * guard is enforced client-side too.
 */
export async function setCreator(
    id: string,
    body: { value: string | null; mode?: CreatorEditMode },
): Promise<SetCreatorResponse> {
    const {data} = await apiClient.post<SetCreatorResponse>(`/api/authenticated/pictures/${id}/creator`, body)
    return data
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
    /** Fields to empty: owned pictures null the column, received-local pictures get a `null` override
     *  claim (10 §6.3). For propose-to-owner this folds into a clear. */
    empty?: ExifField[]
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

/** Body for the batch creator edit endpoint (`PATCH /pictures/creator`, feature 26). */
export interface BatchCreatorBody {
    selection: PictureSelection
    /** New credit; `null`/blank resets owned pictures to the owner default and clears received overrides. */
    value: string | null
    dry_run?: boolean
}

/** Batch-set the creator over a selection (feature 26). Owned pictures get the authoritative
 *  `creator` (re-announced), received pictures the recipient-local `creator_override`. `dry_run`
 *  returns the owned (`edited`) vs received (`local_override`) breakdown. */
export async function batchSetCreator(body: BatchCreatorBody & { dry_run: true }): Promise<BatchDryRun>
export async function batchSetCreator(body: BatchCreatorBody): Promise<BatchDryRun>
export async function batchSetCreator(body: BatchCreatorBody): Promise<BatchDryRun> {
    const {data} = await apiClient.patch<BatchDryRun>('/api/authenticated/pictures/creator', body)
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

/**
 * Copy ("rescue") a received (or owned) picture into the caller's library as a new, independent
 * owned picture (feature 11 §3). Returns the new picture id. The bytes are copied server-side and
 * `gen_thumbnail` fills hashes/thumbnails asynchronously.
 */
export async function copyPicture(id: string): Promise<{ id: string }> {
    const {data} = await apiClient.post<{ id: string }>(`/api/authenticated/pictures/${id}/copy`)
    return data
}

/** One row of a picture's content-dedup group (feature 11 §5.5). */
export interface PictureCopy {
    id: string
    filename: string | null
    content_hash: string | null
    file_hash: string | null
    /** Dedup state: the visible survivor (`live`), the trash representative (`manual`), or hidden. */
    state: 'live' | 'manual' | 'boomerang' | 'content_dedupe' | 'deleted'
    updated_at: string
    owned: boolean
    owner_username: string | null
    owner_instance: string | null
    owner_deleted_at: string | null
    copy_source_owner_username: string | null
    copy_source_owner_instance: string | null
    copy_source_picture_id: string | null
}

/** List the content-dedup group of a picture (survivor + hidden siblings, feature 11 §5.5). */
export async function getPictureCopies(id: string): Promise<PictureCopy[]> {
    const {data} = await apiClient.get<{ copies: PictureCopy[] }>(`/api/authenticated/pictures/${id}/copies`)
    return data.copies
}

/** Make this picture the live survivor of its content-dedup group (hides the others). */
export async function keepCopy(id: string): Promise<void> {
    await apiClient.post(`/api/authenticated/pictures/${id}/copies/keep`)
}

export async function getJob(id: string): Promise<Job> {
    const {data} = await apiClient.get<Job>(`/api/authenticated/jobs/${id}`)
    return data
}

const PRESIGN_BATCH_SIZE = 100

/**
 * Batch-presign upload slots. Each file may carry its SHA-256 (`file_hash`) so the backend can
 * deduplicate against the user's existing owned pictures — a hit comes back with `duplicate: true`,
 * a `null` `presigned_url`, and the existing `picture_id`. `initialTags` are assigned to any such
 * deduplicated pictures server-side (new files get their tags later, on `complete`).
 *
 * The backend caps a single request at 100 files, so larger uploads are chunked.
 */
export async function beginUploadBatch(
    files: BatchUploadFileInput[],
    initialTags?: string[],
    uploadLabel?: string,
): Promise<UploadSlot[]> {
    const slots: UploadSlot[] = []
    for (let i = 0; i < files.length; i += PRESIGN_BATCH_SIZE) {
        const chunk = files.slice(i, i + PRESIGN_BATCH_SIZE)
        const {data} = await apiClient.post<UploadSlot[]>('/api/authenticated/pictures/uploads/batch', {
            files: chunk,
            initial_tags: initialTags?.length ? initialTags : undefined,
            upload_label: uploadLabel,
        })
        slots.push(...data)
    }
    return slots
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
