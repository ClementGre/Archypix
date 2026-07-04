import {apiClient} from '@/api/client'
import type {
    AdminJobResponse,
    AdminUserResponse,
    ConsistencyCheck,
    ErroredShareResponse,
    FederationInstanceResponse,
    InstanceHealth,
    InstanceStats,
    JobStatus,
    JobType,
    StorageAuditResponse,
    UserSharesResponse,
    UserStats,
} from '@/lib/types'

export async function getInstanceHealth(): Promise<InstanceHealth> {
    const {data} = await apiClient.get('/api/admin/instance')
    return data
}

export async function getInstanceStats(): Promise<InstanceStats> {
    const {data} = await apiClient.get('/api/admin/stats')
    return data
}

export async function getConsistencyCheck(): Promise<ConsistencyCheck> {
    const {data} = await apiClient.get('/api/admin/consistency')
    return data
}

export async function listAdminUsers(): Promise<AdminUserResponse[]> {
    const {data} = await apiClient.get('/api/admin/users')
    return data
}

export async function createAdminUser(body: {
    username: string
    email: string
    display_name: string
    password: string
    is_admin?: boolean
}): Promise<AdminUserResponse> {
    const {data} = await apiClient.post('/api/admin/users', body)
    return data
}

export async function updateAdminUser(
    id: string,
    body: { display_name?: string; is_admin?: boolean; storage_quota_bytes?: number | null },
): Promise<AdminUserResponse> {
    const {data} = await apiClient.patch(`/api/admin/users/${id}`, body)
    return data
}

export async function deleteAdminUser(id: string): Promise<void> {
    await apiClient.delete(`/api/admin/users/${id}`)
}

export async function getUserStats(id: string): Promise<UserStats> {
    const {data} = await apiClient.get(`/api/admin/users/${id}/stats`)
    return data
}

export async function getUserShares(id: string): Promise<UserSharesResponse> {
    const {data} = await apiClient.get(`/api/admin/users/${id}/shares`)
    return data
}

/** The S3 truth check (feature 22 §8.3) — per-bucket object counts/bytes + DB-vs-S3 drift. */
export async function getUserStorageAudit(id: string): Promise<StorageAuditResponse> {
    const {data} = await apiClient.get(`/api/admin/users/${id}/storage-audit`)
    return data
}

export async function wakeUserPipeline(id: string): Promise<void> {
    await apiClient.post(`/api/admin/users/${id}/pipeline/wake`)
}

export interface ListJobsParams {
    status?: JobStatus
    type?: JobType
    user_id?: string
    limit?: number
    offset?: number
}

export async function listAdminJobs(params: ListJobsParams = {}): Promise<AdminJobResponse[]> {
    const {data} = await apiClient.get('/api/admin/jobs', {params})
    return data
}

export async function getStaleJobs(): Promise<AdminJobResponse[]> {
    const {data} = await apiClient.get('/api/admin/jobs/stale')
    return data
}

export async function resetJob(id: string): Promise<AdminJobResponse> {
    const {data} = await apiClient.post(`/api/admin/jobs/${id}/reset`)
    return data
}

export async function cancelJob(id: string): Promise<AdminJobResponse> {
    const {data} = await apiClient.post(`/api/admin/jobs/${id}/cancel`)
    return data
}

export async function getErroredShares(): Promise<ErroredShareResponse[]> {
    const {data} = await apiClient.get('/api/admin/shares/errored')
    return data
}

export async function forceReconcileShare(id: string): Promise<void> {
    await apiClient.post(`/api/admin/shares/outgoing/${id}/force-reconcile`)
}

export async function listFederationInstances(): Promise<FederationInstanceResponse[]> {
    const {data} = await apiClient.get('/api/admin/federation/instances')
    return data
}

/**
 * Bulk (re)enqueue `gen_thumbnail` jobs (feature 11). `scope: 'missing'` targets owned pictures with
 * a thumbnailable MIME, no thumbnail, older than 30 min (failed/never-run jobs); `'all'` targets the
 * whole owned library (e.g. to recompute `content_hash`). `reextract_exif` also re-extracts EXIF.
 */
export async function regenerateThumbnails(body: {
    scope: 'missing' | 'all'
    reextract_exif?: boolean
}): Promise<{ enqueued: number }> {
    const {data} = await apiClient.post('/api/admin/pictures/regenerate-thumbnails', body)
    return data
}
