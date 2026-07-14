import axios from 'axios'
import {apiClient} from '@/api/client'
import {resolveConnection} from '@/api/resolve'
import type {PictureListResponse, PictureVariant} from '@/lib/types'

// ── Types ────────────────────────────────────────────────────────────────────

export interface PublicPermissions {
    allow_originals: boolean
    allow_upload: boolean
    allow_share_back: boolean
    conv_allow_exif_edit: boolean
    conv_future: boolean
}

export interface PublicShareMeta {
    name: string
    message: string | null
    /** The owner's `@username:global_domain` identity handle. */
    owner_display: string
    /** The album's covered tag (wire/ltree form) — used to detect an existing incoming share. */
    tag_path: string
    permissions: PublicPermissions
    picture_count: number
    requires_password: boolean
    expires_at: string | null
    /** `true` ⇒ thumbnails only, EXIF/GPS stripped. */
    view_only: boolean
}

export interface PublicPictureDetail {
    id: string
    filename: string | null
    mime_type: string | null
    file_size: number | null
    width: number | null
    height: number | null
    blurhash: string | null
    orientation: number | null
    ingested_at: string
    creator: string
    /** Present only on non-view-only shares. */
    captured_at?: string | null
    gps_lat?: number | null
    gps_lng?: number | null
    gps_alt?: number | null
    exif_data?: Record<string, unknown> | null
}

export interface PublicUploadSlot {
    picture_id: string
    presigned_url: string | null
    /** A dedup hit — the bytes already exist in the owner's library; not stored. */
    rejected: boolean
}

/** The owner-facing view of a public share (includes the secret token to build the link). */
export interface PublicShareSummary {
    id: string
    tag_path: string
    name: string
    message: string | null
    token: string
    has_password: boolean
    expires_at: string | null
    permissions: PublicPermissions
    status: 'active' | 'revoked'
    created_at: string
    revoked_at: string | null
    derived_share_count: number
    contribution_count: number
}

export interface PublicShareBody {
    tag_path: string
    name: string
    message?: string | null
    password?: string | null
    keep_password?: boolean
    expires_at?: string | null
    allow_originals: boolean
    allow_upload: boolean
    allow_share_back: boolean
    conv_allow_exif_edit: boolean
    conv_future: boolean
}

export interface PublicRevokeOutcome {
    revoked: boolean
    derived_revoked: number
    contributions_trashed: number
}

// ── Public (unauthenticated, token-gated) surface ─────────────────────────────

/** A bare axios instance for the public surface — no auth interceptors. */
const pub = axios.create()

function auth(sessionJwt?: string | null) {
    return sessionJwt ? {headers: {Authorization: `Bearer ${sessionJwt}`}} : {}
}

/** Resolve the owner backend base URL for a public link's `(username, global_domain)`. */
export async function resolvePublicBackend(username: string, globalDomain: string): Promise<string> {
    const conn = await resolveConnection(username, globalDomain)
    return conn.backendUrl
}

function base(backendUrl: string, token: string): string {
    return `${backendUrl}/api/public/shares/${encodeURIComponent(token)}`
}

export async function getPublicMeta(backendUrl: string, token: string): Promise<PublicShareMeta> {
    const {data} = await pub.get<PublicShareMeta>(base(backendUrl, token))
    return data
}

export async function unlockPublicShare(backendUrl: string, token: string, password: string): Promise<string> {
    const {data} = await pub.post<{ token: string }>(`${base(backendUrl, token)}/unlock`, {password})
    return data.token
}

export async function listPublicPictures(
    backendUrl: string,
    token: string,
    opts: { page: number; page_size?: number; thumbnail?: PictureVariant; sessionJwt?: string | null },
): Promise<PictureListResponse> {
    const {data} = await pub.get<PictureListResponse>(`${base(backendUrl, token)}/pictures`, {
        params: {page: opts.page, page_size: opts.page_size ?? 50, thumbnail: opts.thumbnail ?? 'medium'},
        ...auth(opts.sessionJwt),
    })
    return data
}

export async function getPublicPictureUrl(
    backendUrl: string,
    token: string,
    pictureId: string,
    variant: PictureVariant,
    sessionJwt?: string | null,
): Promise<string | null> {
    const {data} = await pub.get<{ url: string | null }>(`${base(backendUrl, token)}/pictures/${pictureId}/url`, {
        params: {variant},
        ...auth(sessionJwt),
    })
    return data.url
}

/** Download a covered picture's original under its filename (presign via the public endpoint + blob). */
export async function downloadPublicOriginal(
    backendUrl: string,
    token: string,
    pictureId: string,
    filename: string | null,
    sessionJwt?: string | null,
): Promise<void> {
    const url = await getPublicPictureUrl(backendUrl, token, pictureId, 'original', sessionJwt)
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
        // Cross-origin fetch blocked (S3 CORS) — open the URL; downloads under the S3 key.
        window.open(url, '_blank')
    }
}

export async function getPublicPictureDetail(
    backendUrl: string,
    token: string,
    pictureId: string,
    sessionJwt?: string | null,
): Promise<PublicPictureDetail> {
    const {data} = await pub.get<PublicPictureDetail>(`${base(backendUrl, token)}/pictures/${pictureId}`, auth(sessionJwt))
    return data
}

export interface PublicAggregateResponse {
    count?: number
    total_file_size?: number

    [key: string]: unknown
}

export async function publicAggregate(
    backendUrl: string,
    token: string,
    includeIds: string[],
    sections: string[],
    sessionJwt?: string | null,
): Promise<PublicAggregateResponse> {
    const {data} = await pub.post<PublicAggregateResponse>(
        `${base(backendUrl, token)}/aggregate`,
        {include_ids: includeIds, sections},
        auth(sessionJwt),
    )
    return data
}

export interface PublicUploadFile {
    filename: string
    file_hash?: string | null
    size?: number | null
}

export async function publicUploadBatch(
    backendUrl: string,
    token: string,
    contributorName: string,
    files: PublicUploadFile[],
): Promise<PublicUploadSlot[]> {
    const {data} = await pub.post<PublicUploadSlot[]>(`${base(backendUrl, token)}/uploads`, {
        contributor_name: contributorName,
        files,
    })
    return data
}

export interface PublicCompleteBody {
    contributor_name: string
    mime_type?: string | null
    file_size?: number | null
    file_hash?: string | null
    width?: number | null
    height?: number | null
}

export async function publicCompleteUpload(
    backendUrl: string,
    token: string,
    pictureId: string,
    body: PublicCompleteBody,
): Promise<{ id: string }> {
    const {data} = await pub.post<{ id: string }>(`${base(backendUrl, token)}/uploads/${pictureId}/complete`, body)
    return data
}

// ── Convert (authenticated visitor, on their own backend via apiClient) ────────

export async function saveCopyFromPublic(body: {
    owner_username: string
    owner_instance: string
    token: string
    picture_id: string
}): Promise<{ id: string }> {
    const {data} = await apiClient.post('/api/authenticated/shares/public/save-copy', body)
    return data
}

export async function subscribeToPublic(body: {
    owner_username: string
    owner_instance: string
    token: string
}): Promise<{ outgoing_share_id: string; name: string; tag_path: string; allow_share_back: boolean }> {
    const {data} = await apiClient.post('/api/authenticated/shares/public/subscribe', body)
    return data
}

// ── Management (owner, authenticated apiClient) ────────────────────────────────

export async function listPublicShares(): Promise<PublicShareSummary[]> {
    const {data} = await apiClient.get<PublicShareSummary[]>('/api/authenticated/shares/public')
    return data
}

export async function createPublicShare(body: PublicShareBody): Promise<PublicShareSummary> {
    const {data} = await apiClient.post<PublicShareSummary>('/api/authenticated/shares/public', body)
    return data
}

export async function updatePublicShare(id: string, body: PublicShareBody): Promise<PublicShareSummary> {
    const {data} = await apiClient.patch<PublicShareSummary>(`/api/authenticated/shares/public/${id}`, body)
    return data
}

export async function revokePublicShare(
    id: string,
    body: { cascade_derived: boolean; trash_contributions: boolean },
): Promise<PublicRevokeOutcome> {
    const {data} = await apiClient.post<PublicRevokeOutcome>(`/api/authenticated/shares/public/${id}/revoke`, body)
    return data
}

export async function trashContributions(id: string, contributor?: string | null): Promise<{ trashed: number }> {
    const {data} = await apiClient.post(`/api/authenticated/shares/public/${id}/contributions/trash`, {contributor})
    return data
}

/** Build the shareable public URL for a link (opened by any correctly-CORS'd frontend). */
export function publicShareUrl(globalDomain: string, username: string, token: string): string {
    return `${window.location.origin}/s/${encodeURIComponent(globalDomain)}/${encodeURIComponent(username)}/${encodeURIComponent(token)}`
}
