import axios, {type AxiosError, type InternalAxiosRequestConfig} from 'axios'
import {useResolverAuthStore} from '@/stores/resolverAuth'
import type {ConfigMatrixPatchResult, FieldMeta, Invite, ResolverBackend, ResolverOverview, ResolverSession, RoutineInfo,} from '@/lib/types'

/**
 * The fleet dashboard talks to a resolver's `api_url` (`…/archypix-resolver`), chosen at login and
 * held in `resolverAuth` (feature 25) — no longer hard-wired to the global domain. This is a second
 * axios instance parallel to `apiClient`, bearing the `ResolverAdminSession` token + its own
 * 401→refresh, so the fleet dashboard never needs a user token on any backend.
 */
function resolverBase(): string {
    const url = useResolverAuthStore.getState().resolverUrl
    if (!url) throw new Error('No resolver selected.')
    return url
}

let refreshPromise: Promise<string> | null = null

async function refreshSession(): Promise<string> {
    const {refreshToken} = useResolverAuthStore.getState()
    if (!refreshToken) throw new Error('no resolver refresh token')
    const {data} = await axios.post<ResolverSession>(`${resolverBase()}/api/resolver-admin/refresh`, {
        refresh_token: refreshToken,
    })
    useResolverAuthStore.getState().setSession({
        sessionToken: data.session_token,
        refreshToken: data.refresh_token,
        expiresInSecs: data.expires_in_secs,
    })
    return data.session_token
}

/**
 * Install the resolver-operator bearer + single-flight 401→refresh on an axios instance. Shared by
 * `resolverClient` and the per-instance proxy clients (feature 24 §5) so both ride one session.
 */
export function installResolverAuth(instance: ReturnType<typeof axios.create>) {
    instance.interceptors.request.use((config) => {
        const {sessionToken, resolverUrl} = useResolverAuthStore.getState()
        // The connected resolver base is dynamic (feature 25) — read it per request. Reject rather than
        // fall back to the app origin (which would answer with index.html and poison query caches).
        if (!resolverUrl) return Promise.reject(new Error('No resolver selected.'))
        config.baseURL = resolverUrl
        if (sessionToken) config.headers.Authorization = `Bearer ${sessionToken}`
        return config
    })
    instance.interceptors.response.use(
        (r) => r,
        async (error: AxiosError) => {
            const original = error.config as (InternalAxiosRequestConfig & { _retry?: boolean }) | undefined
            const isAuthCall = original?.url?.includes('/resolver-admin/refresh') || original?.url?.includes('/resolver-admin/login')
            if (error.response?.status === 401 && original && !original._retry && !isAuthCall) {
                original._retry = true
                try {
                    if (!refreshPromise) refreshPromise = refreshSession().finally(() => (refreshPromise = null))
                    const token = await refreshPromise
                    original.headers.Authorization = `Bearer ${token}`
                    return instance(original)
                } catch {
                    useResolverAuthStore.getState().clear()
                }
            }
            return Promise.reject(error)
        },
    )
    return instance
}

export const resolverClient = installResolverAuth(axios.create())

// ── Auth ──────────────────────────────────────────────────────────────────────

export async function login(token: string): Promise<ResolverSession> {
    const {data} = await axios.post<ResolverSession>(`${resolverBase()}/api/resolver-admin/login`, {token})
    return data
}

export async function refresh(refresh_token: string): Promise<ResolverSession> {
    const {data} = await axios.post<ResolverSession>(`${resolverBase()}/api/resolver-admin/refresh`, {refresh_token})
    return data
}

// ── Fleet monitoring ────────────────────────────────────────────────────────────

export async function getOverview(): Promise<ResolverOverview> {
    const {data} = await resolverClient.get<ResolverOverview>('/api/resolver-admin/overview')
    return data
}

export async function getBackends(): Promise<ResolverBackend[]> {
    const {data} = await resolverClient.get<ResolverBackend[]>('/api/resolver-admin/backends')
    return data
}

/** Dry-run placement: which backend the next (un-pinned) signup would land on (`null` if none eligible). */
export async function getNextBackend(): Promise<string | null> {
    const {data} = await resolverClient.get<{ back_domain: string | null }>('/api/resolver-admin/next-backend')
    return data?.back_domain ?? null
}

export async function setCapacity(
    backDomain: string,
    body: { accepting_registrations: boolean; max_users: number | null },
): Promise<void> {
    await resolverClient.patch(`/api/resolver-admin/backends/${encodeURIComponent(backDomain)}/capacity`, body)
}

// ── Resolver's own settings ───────────────────────────────────────────────────────

export async function getSettings(): Promise<FieldMeta[]> {
    const {data} = await resolverClient.get<FieldMeta[]>('/api/resolver-admin/settings')
    return data
}

export async function patchSetting(key: string, value: unknown): Promise<FieldMeta[]> {
    const {data} = await resolverClient.patch<FieldMeta[]>('/api/resolver-admin/settings', {key, value})
    return data
}

export async function resetSetting(key: string): Promise<FieldMeta[]> {
    const {data} = await resolverClient.delete<FieldMeta[]>(`/api/resolver-admin/settings/${encodeURIComponent(key)}`)
    return data
}

// ── Invites ────────────────────────────────────────────────────────────────────

export async function listInvites(): Promise<Invite[]> {
    const {data} = await resolverClient.get<Invite[]>('/api/resolver-admin/invites')
    return data
}

export async function mintInvite(body: {
    max_uses: number | null
    expires_in_days: number | null
    instance_pin: string | null
}): Promise<Invite> {
    const {data} = await resolverClient.post<Invite>('/api/resolver-admin/invites', body)
    return data
}

export async function revokeInvite(code: string): Promise<void> {
    await resolverClient.delete(`/api/resolver-admin/invites/${encodeURIComponent(code)}`)
}

// ── Routines ───────────────────────────────────────────────────────────────────

export async function getRoutines(): Promise<RoutineInfo[]> {
    const {data} = await resolverClient.get<RoutineInfo[]>('/api/resolver-admin/routines')
    return data
}

export async function triggerRoutine(name: string): Promise<void> {
    await resolverClient.post(`/api/resolver-admin/routines/${encodeURIComponent(name)}/trigger`)
}

// ── Config matrix (fan-out) ───────────────────────────────────────────────────────

/** Raw fan-out: `{ [back_domain]: FieldMeta[] | { error } }`. */
export type ConfigMatrixResponse = Record<string, FieldMeta[] | { error: string }>

export async function getConfigMatrix(): Promise<ConfigMatrixResponse> {
    const {data} = await resolverClient.get<ConfigMatrixResponse>('/api/resolver-admin/config-matrix')
    return data
}

export async function patchConfigMatrix(body: {
    key: string
    value: unknown
    targets?: string[]
}): Promise<Record<string, ConfigMatrixPatchResult>> {
    const {data} = await resolverClient.patch<Record<string, ConfigMatrixPatchResult>>(
        '/api/resolver-admin/config-matrix',
        body,
    )
    return data
}

// ── Per-instance drill-down proxy ─────────────────────────────────────────────────

/** Reach a backend's own `/api/admin/{path}` through the resolver (delegation replay). */
export async function proxy<T = unknown>(
    backDomain: string,
    method: 'get' | 'post' | 'patch' | 'delete',
    path: string,
    body?: unknown,
): Promise<T> {
    const url = `/api/resolver-admin/instances/${encodeURIComponent(backDomain)}/api/admin/${path.replace(/^\//, '')}`
    const {data} = await resolverClient.request<T>({url, method, data: body})
    return data
}
