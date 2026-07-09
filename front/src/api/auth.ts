import axios from 'axios'
import {apiClient} from './client'
import {getResolverInfo, resolveConnection} from './resolve'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {type AuthUser, useAuthStore} from '@/stores/auth'

export interface RegisterPayload {
    username: string
    email: string
    display_name: string
    password: string
    /** Invite code from a `/register?invite=…` link (required in invite modes, optional pinning in open). */
    invite_code?: string
}

/**
 * Log in to @username:instance. Bootstraps the instance (`/archypix-resolver/info`), resolves the
 * hosting backend (directly when resolver-fronted, else the standalone backend), authenticates there,
 * stores the session (incl. whether a fleet dashboard exists), then loads the profile.
 */
export async function login(username: string, password: string, instance: string): Promise<AuthUser> {
    const {backendUrl, isResolver, resolverUrl} = await resolveConnection(username, instance)
    const {data} = await axios.post(`${backendUrl}/api/auth/login`, {username, password})
    useAuthStore.getState().setSession({
        accessToken: data.access_token,
        refreshToken: data.refresh_token,
        backendUrl,
        instance,
        isResolver,
        resolverUrl,
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
 * Bootstraps the domain (`/archypix-resolver/info`) to learn where the registration surface lives,
 * then POSTs to `{api_url}/api/public/register` — served by a standalone backend directly and by a
 * resolver under its `/archypix-resolver` prefix, so the frontend needn't know the topology.
 */
export async function register(payload: RegisterPayload, domain: string = GLOBAL_DOMAIN): Promise<void> {
    const {api_url} = await getResolverInfo(domain)
    await axios.post(`${api_url}/api/public/register`, payload)
}
