import {create} from 'zustand'

/**
 * Resolver operator session (feature 24 §2/§4) — a *separate* identity from user auth. The operator
 * token exchanges for a short `ResolverAdminSession` JWT + a rotating refresh token, held here (never
 * mixed into `stores/auth.ts`). Persisted so a reload keeps the fleet dashboard open.
 */
interface ResolverAuthState {
    sessionToken: string | null
    refreshToken: string | null
    /** Unix ms when the session JWT expires (used to schedule a background refresh). */
    expiresAt: number | null

    setSession: (s: { sessionToken: string; refreshToken: string; expiresInSecs: number }) => void
    clear: () => void
}

const LS_KEY = 'archypix_resolver_admin'

type Persisted = Pick<ResolverAuthState, 'sessionToken' | 'refreshToken' | 'expiresAt'>

const EMPTY: Persisted = {sessionToken: null, refreshToken: null, expiresAt: null}

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

export const useResolverAuthStore = create<ResolverAuthState>((set) => ({
    ...load(),

    setSession: ({sessionToken, refreshToken, expiresInSecs}) => {
        const next = {sessionToken, refreshToken, expiresAt: Date.now() + expiresInSecs * 1000}
        set(next)
        persist(next)
    },
    clear: () => {
        set(EMPTY)
        localStorage.removeItem(LS_KEY)
    },
}))
