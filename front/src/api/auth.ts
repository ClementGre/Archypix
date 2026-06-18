import axios from 'axios'
import {apiClient} from './client'
import {resolveBackendUrl} from './webfinger'
import {GLOBAL_DOMAIN, originFor} from '@/lib/constants'
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
 * Register a new user. Defaults to the configured global domain, but a custom
 * `domain` (e.g. a self-hosted instance) can be targeted the same way login can.
 *
 * Always POSTs to `{domain}/api/public/register`: both a standalone backend and the
 * resolver expose this path (the resolver mirrors it onto its registration
 * handler), so the frontend doesn't need to know which topology it's talking to.
 */
export async function register(payload: RegisterPayload, domain: string = GLOBAL_DOMAIN): Promise<void> {
    await axios.post(`${originFor(domain)}/api/public/register`, payload)
}
