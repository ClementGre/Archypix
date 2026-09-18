import axios, {type AxiosError, type InternalAxiosRequestConfig} from 'axios'
import {useAuthStore} from '@/stores/auth'

/**
 * Shared axios instance for authenticated calls. The base URL is the backend
 * resolved for the logged-in user (federated model) — taken from the auth store
 * at request time, so it always targets the right instance.
 */
export const apiClient = axios.create()

apiClient.interceptors.request.use((config) => {
    const {backendUrl, accessToken} = useAuthStore.getState()
    if (!config.baseURL && backendUrl) config.baseURL = backendUrl
    if (accessToken) config.headers.Authorization = `Bearer ${accessToken}`
    return config
})

// De-duplicate concurrent refreshes: many 401s share one refresh round-trip.
let refreshPromise: Promise<string> | null = null

async function refreshAccessToken(): Promise<string> {
    const {refreshToken, backendUrl} = useAuthStore.getState()
    if (!refreshToken || !backendUrl) throw new Error('no refresh token')
    const {data} = await axios.post(`${backendUrl}/api/auth/refresh`, {refresh_token: refreshToken})
    useAuthStore.getState().setTokens({accessToken: data.access_token, refreshToken: data.refresh_token})
    return data.access_token as string
}

/** Seconds of remaining validity below which a flush refreshes before sending (feature 34 §4.1). */
const TOKEN_REFRESH_MARGIN_S = 60

function expiresWithin(token: string, seconds: number): boolean {
    try {
        const payload = JSON.parse(atob(token.split('.')[1] ?? ''))
        return typeof payload.exp === 'number' && payload.exp - Date.now() / 1000 < seconds
    } catch {
        return false // an unreadable token is left to the ordinary 401 → refresh path
    }
}

/**
 * Return an access token that is safe to put on a raw `fetch` — one that escapes the response
 * interceptor's 401 → refresh → retry, and at `pagehide` has no document left to retry in. Refreshes
 * proactively when the current token is nearly expired; otherwise returns it unchanged.
 */
export async function freshAccessToken(): Promise<string | null> {
    const {accessToken} = useAuthStore.getState()
    if (!accessToken || !expiresWithin(accessToken, TOKEN_REFRESH_MARGIN_S)) return accessToken
    try {
        if (!refreshPromise) {
            refreshPromise = refreshAccessToken().finally(() => {
                refreshPromise = null
            })
        }
        return await refreshPromise
    } catch {
        return accessToken
    }
}

apiClient.interceptors.response.use(
    (response) => response,
    async (error: AxiosError) => {
        const original = error.config as (InternalAxiosRequestConfig & { _retry?: boolean }) | undefined
        if (error.response?.status === 401 && original && !original._retry) {
            original._retry = true
            try {
                if (!refreshPromise) {
                    refreshPromise = refreshAccessToken().finally(() => {
                        refreshPromise = null
                    })
                }
                const token = await refreshPromise
                original.headers.Authorization = `Bearer ${token}`
                return apiClient(original)
            } catch {
                // Refresh failed — clear session. ProtectedRoute reacts and redirects to /login.
                useAuthStore.getState().clear()
            }
        }
        return Promise.reject(error)
    },
)

/** Extract a human-readable message from an axios error for toasts. */
export function apiErrorMessage(error: unknown, fallback = 'Something went wrong'): string {
    if (axios.isAxiosError(error)) {
        const data = error.response?.data as { error?: string; message?: string } | string | undefined
        if (typeof data === 'string' && data) return data
        if (data && typeof data === 'object') return data.error || data.message || error.message
        return error.message
    }
    if (error instanceof Error) return error.message
    return fallback
}
