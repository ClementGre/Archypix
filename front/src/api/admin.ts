import type {AxiosInstance} from 'axios'
import {apiClient} from '@/api/client'
import type {
    AdminJobResponse,
    AdminUserResponse,
    ConsistencyCheck,
    ErroredShareResponse,
    FederationInstanceResponse,
    FieldMeta,
    InstanceHealth,
    InstanceStats,
    Invite,
    JobStatus,
    JobType,
    RateLimitsResponse,
    RoutineInfo,
    StorageAuditResponse,
    UserSharesResponse,
    UserStats,
} from '@/lib/types'

// Every admin call takes the axios instance to use (feature 24 §5): the user's `apiClient` for the
// direct `/admin`, or a resolver-proxy instance for a per-backend fleet drill-down. Defaults to
// `apiClient` so non-fleet callers are unaffected.

export async function getInstanceHealth(client: AxiosInstance = apiClient): Promise<InstanceHealth> {
    const {data} = await client.get('/api/admin/instance')
    return data
}

export async function getInstanceStats(client: AxiosInstance = apiClient): Promise<InstanceStats> {
    const {data} = await client.get('/api/admin/stats')
    return data
}

export async function getConsistencyCheck(client: AxiosInstance = apiClient): Promise<ConsistencyCheck> {
    const {data} = await client.get('/api/admin/consistency')
    return data
}

export async function listAdminUsers(client: AxiosInstance = apiClient): Promise<AdminUserResponse[]> {
    const {data} = await client.get('/api/admin/users')
    return data
}

export async function createAdminUser(client: AxiosInstance, body: {
    username: string
    email: string
    display_name: string
    password: string
    is_admin?: boolean
}): Promise<AdminUserResponse> {
    const {data} = await client.post('/api/admin/users', body)
    return data
}

export async function updateAdminUser(
    client: AxiosInstance,
    id: string,
    body: { display_name?: string; is_admin?: boolean; storage_quota_bytes?: number | null },
): Promise<AdminUserResponse> {
    const {data} = await client.patch(`/api/admin/users/${id}`, body)
    return data
}

export async function deleteAdminUser(client: AxiosInstance, id: string): Promise<void> {
    await client.delete(`/api/admin/users/${id}`)
}

export async function getUserStats(client: AxiosInstance, id: string): Promise<UserStats> {
    const {data} = await client.get(`/api/admin/users/${id}/stats`)
    return data
}

export async function getUserShares(client: AxiosInstance, id: string): Promise<UserSharesResponse> {
    const {data} = await client.get(`/api/admin/users/${id}/shares`)
    return data
}

/** The S3 truth check (feature 22 §8.3) — per-bucket object counts/bytes + DB-vs-S3 drift. */
export async function getUserStorageAudit(client: AxiosInstance, id: string): Promise<StorageAuditResponse> {
    const {data} = await client.get(`/api/admin/users/${id}/storage-audit`)
    return data
}

export async function wakeUserPipeline(client: AxiosInstance, id: string): Promise<void> {
    await client.post(`/api/admin/users/${id}/pipeline/wake`)
}

export interface ListJobsParams {
    status?: JobStatus
    type?: JobType
    user_id?: string
    limit?: number
    offset?: number
}

export async function listAdminJobs(client: AxiosInstance = apiClient, params: ListJobsParams = {}): Promise<AdminJobResponse[]> {
    const {data} = await client.get('/api/admin/jobs', {params})
    return data
}

export async function getStaleJobs(client: AxiosInstance = apiClient): Promise<AdminJobResponse[]> {
    const {data} = await client.get('/api/admin/jobs/stale')
    return data
}

export async function resetJob(client: AxiosInstance, id: string): Promise<AdminJobResponse> {
    const {data} = await client.post(`/api/admin/jobs/${id}/reset`)
    return data
}

export async function cancelJob(client: AxiosInstance, id: string): Promise<AdminJobResponse> {
    const {data} = await client.post(`/api/admin/jobs/${id}/cancel`)
    return data
}

export async function getErroredShares(client: AxiosInstance = apiClient): Promise<ErroredShareResponse[]> {
    const {data} = await client.get('/api/admin/shares/errored')
    return data
}

export async function forceReconcileShare(client: AxiosInstance, id: string): Promise<void> {
    await client.post(`/api/admin/shares/outgoing/${id}/force-reconcile`)
}

export async function listFederationInstances(client: AxiosInstance = apiClient): Promise<FederationInstanceResponse[]> {
    const {data} = await client.get('/api/admin/federation/instances')
    return data
}

/**
 * Bulk (re)enqueue `gen_thumbnail` jobs (feature 11). `scope: 'missing'` targets owned pictures with
 * a thumbnailable MIME, no thumbnail, older than 30 min (failed/never-run jobs); `'all'` targets the
 * whole owned library (e.g. to recompute `content_hash`). `reextract_exif` also re-extracts EXIF.
 */
export async function regenerateThumbnails(client: AxiosInstance, body: {
    scope: 'missing' | 'all'
    reextract_exif?: boolean
}): Promise<{ enqueued: number }> {
    const {data} = await client.post('/api/admin/pictures/regenerate-thumbnails', body)
    return data
}

// ── Rate limiting observability (feature 28 §9.3) ────────────────────────────────

export async function getRateLimits(client: AxiosInstance = apiClient): Promise<RateLimitsResponse> {
    const {data} = await client.get<RateLimitsResponse>('/api/admin/rate-limits')
    return data
}

// ── Runtime settings (feature 23 §4.5) ──────────────────────────────────────────

export async function getAdminSettings(client: AxiosInstance = apiClient): Promise<FieldMeta[]> {
    const {data} = await client.get<FieldMeta[]>('/api/admin/settings')
    return data
}

export async function patchAdminSetting(client: AxiosInstance, key: string, value: unknown): Promise<FieldMeta[]> {
    const {data} = await client.patch<FieldMeta[]>('/api/admin/settings', {key, value})
    return data
}

export async function resetAdminSetting(client: AxiosInstance, key: string): Promise<FieldMeta[]> {
    const {data} = await client.delete<FieldMeta[]>(`/api/admin/settings/${encodeURIComponent(key)}`)
    return data
}

// ── Routines (feature 23 §5.2) ───────────────────────────────────────────────────

export async function getAdminRoutines(client: AxiosInstance = apiClient): Promise<RoutineInfo[]> {
    const {data} = await client.get<RoutineInfo[]>('/api/admin/routines')
    return data
}

export async function triggerAdminRoutine(client: AxiosInstance, name: string): Promise<void> {
    await client.post(`/api/admin/routines/${encodeURIComponent(name)}/trigger`)
}

// ── Invites (feature 24) — all local invites, grouped by creator ──────────────────

export async function listAllAdminInvites(client: AxiosInstance = apiClient): Promise<Invite[]> {
    const {data} = await client.get<Invite[]>('/api/admin/invites')
    return data
}

export async function revokeAnyInvite(client: AxiosInstance, code: string): Promise<void> {
    await client.delete(`/api/admin/invites/${encodeURIComponent(code)}`)
}
