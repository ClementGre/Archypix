import {create} from 'zustand'

/**
 * Resolver operator session (feature 24 §2/§4) — a *separate* identity from user auth. The operator
 * token exchanges for a short `ResolverAdminSession` JWT + a rotating refresh token, held here (never
 * mixed into `stores/auth.ts`). Persisted so a reload keeps the fleet dashboard open.
 *
 * `resolverUrl` (feature 25) is the resolver's `api_url` (`…/archypix-resolver`) the dashboard is
 * connected to — chosen at login, so an operator can target a resolver other than the default global
 * domain. Every `resolverClient` call reads it as the axios `baseURL` at request time.
 */
interface ResolverAuthState {
    sessionToken: string | null
    refreshToken: string | null
    /** Unix ms when the session JWT expires (used to schedule a background refresh). */
    expiresAt: number | null
    /** The resolver base URL (`…/archypix-resolver`) this session is bound to. */
    resolverUrl: string | null

    setResolverUrl: (url: string) => void
    setSession: (s: { sessionToken: string; refreshToken: string; expiresInSecs: number }) => void
    clear: () => void
}

const LS_KEY = 'archypix_resolver_admin'

type Persisted = Pick<ResolverAuthState, 'sessionToken' | 'refreshToken' | 'expiresAt' | 'resolverUrl'>

const EMPTY: Persisted = {sessionToken: null, refreshToken: null, expiresAt: null, resolverUrl: null}

function load(): Persisted {
    try {
        const raw = localStorage.getItem(LS_KEY)
        if (raw) return {...EMPTY, ...(JSON.parse(raw) as Persisted)}
    } catch {
        // ignore malformed storage
    }
    return EMPTY
}

function persist(s: Persisted) {
    localStorage.setItem(LS_KEY, JSON.stringify(s))
}

export const useResolverAuthStore = create<ResolverAuthState>((set, get) => ({
    ...load(),

    setResolverUrl: (resolverUrl) => {
        set({resolverUrl})
        const {sessionToken, refreshToken, expiresAt} = get()
        persist({sessionToken, refreshToken, expiresAt, resolverUrl})
    },
    setSession: ({sessionToken, refreshToken, expiresInSecs}) => {
        const expiresAt = Date.now() + expiresInSecs * 1000
        set({sessionToken, refreshToken, expiresAt})
        persist({sessionToken, refreshToken, expiresAt, resolverUrl: get().resolverUrl})
    },
    clear: () => {
        // Keep `resolverUrl` so the login form defaults back to the last-used resolver.
        const {resolverUrl} = get()
        set({...EMPTY, resolverUrl})
        persist({...EMPTY, resolverUrl})
    },
}))
