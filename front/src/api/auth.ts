import axios from 'axios'
import {apiClient} from './client'
import {resolveBackendUrl} from './webfinger'
import {GLOBAL_DOMAIN, originFor, REGISTRATION_MODE, REGISTRATION_URL} from '@/lib/constants'
import {type AuthUser, useAuthStore} from '@/stores/auth'

export interface RegisterPayload {
    username: string
    email: string
    display_name: string
    password: string
}

/**
 * Log in to @username:instance. Resolves the hosting backend via WebFinger,
 * authenticates there, stores the session, then loads the profile.
 */
export async function login(username: string, password: string, instance: string): Promise<AuthUser> {
    const backendUrl = await resolveBackendUrl(username, instance)
    const {data} = await axios.post(`${backendUrl}/api/auth/login`, {username, password})
    useAuthStore.getState().setSession({
        accessToken: data.access_token,
        refreshToken: data.refresh_token,
        backendUrl,
        instance,
    })
    const me = await fetchMe()
    useAuthStore.getState().setUser(me)
    return me
}

export async function fetchMe(): Promise<AuthUser> {
    const {data} = await apiClient.get<AuthUser>('/api/auth/me')
    return data
}

export async function logout(): Promise<void> {
    const {refreshToken} = useAuthStore.getState()
    try {
        await apiClient.post('/api/auth/logout', refreshToken ? {refresh_token: refreshToken} : {})
    } catch {
        // Best-effort server-side invalidation; clear locally regardless.
    }
    useAuthStore.getState().clear()
}

/**
 * Register a new user on the configured global domain.
 *
 * - explicit URL override (VITE_REGISTRATION_URL) → POST there verbatim.
 * - `resolver`   → POST {global}/api/register (resolver assigns a backend).
 * - `standalone` → POST {global}/api/public/users (the global domain IS a backend).
 * - `auto`       → try the resolver endpoint; if it 404s (no resolver here),
 *                  fall back to the standalone endpoint on the same domain.
 */
export async function register(payload: RegisterPayload): Promise<void> {
    if (REGISTRATION_URL) {
        await axios.post(REGISTRATION_URL, payload)
        return
    }
    switch (REGISTRATION_MODE) {
        case 'resolver':
            await resolverRegister(payload)
            return
        case 'standalone':
            await standaloneRegister(payload)
            return
        default: {
            try {
                await resolverRegister(payload)
            } catch (error) {
                // A standalone backend has no /api/register route → 404. Anything else
                // (e.g. 400 validation, 409 taken) is a real error and is surfaced.
                if (axios.isAxiosError(error) && error.response?.status === 404) {
                    await standaloneRegister(payload)
                    return
                }
                throw error
            }
        }
    }
}

function resolverRegister(payload: RegisterPayload) {
    return axios.post(`${originFor(GLOBAL_DOMAIN)}/api/register`, payload)
}

function standaloneRegister(payload: RegisterPayload) {
    return axios.post(`${originFor(GLOBAL_DOMAIN)}/api/public/users`, payload)
}
