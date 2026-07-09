import {create} from 'zustand'

export interface AuthUser {
    id: string
    username: string
    email: string
    display_name: string
    is_admin: boolean
}

interface AuthState {
    user: AuthUser | null
    accessToken: string | null
    refreshToken: string | null
    /** Resolved backend base URL (scheme + host) for the logged-in user. */
    backendUrl: string | null
    /** The user's global/identity domain (the part after ':' in @user:domain). */
    instance: string | null
    /** Whether the user's instance is fronted by a resolver (⇒ a fleet dashboard exists, feature 25). */
    isResolver: boolean
    /** The resolver's `api_url` (`…/archypix-resolver`) for the user's instance, when `isResolver`. */
    resolverUrl: string | null

    setSession: (s: {
        accessToken: string
        refreshToken: string
        backendUrl: string
        instance: string
        isResolver: boolean
        resolverUrl: string | null
    }) => void
    setTokens: (t: { accessToken: string; refreshToken: string }) => void
    setUser: (user: AuthUser | null) => void
    clear: () => void
}

const LS_KEY = 'archypix_auth'

type Persisted = Pick<AuthState, 'user' | 'accessToken' | 'refreshToken' | 'backendUrl' | 'instance' | 'isResolver' | 'resolverUrl'>

const EMPTY: Persisted = {
    user: null,
    accessToken: null,
    refreshToken: null,
    backendUrl: null,
    instance: null,
    isResolver: false,
    resolverUrl: null,
}

function load(): Persisted {
    try {
        const raw = localStorage.getItem(LS_KEY)
        if (raw) return {...EMPTY, ...(JSON.parse(raw) as Persisted)}
    } catch {
        // ignore malformed storage
    }
    return EMPTY
}

function persist(state: AuthState) {
    const {user, accessToken, refreshToken, backendUrl, instance, isResolver, resolverUrl} = state
    localStorage.setItem(LS_KEY, JSON.stringify({user, accessToken, refreshToken, backendUrl, instance, isResolver, resolverUrl}))
}

export const useAuthStore = create<AuthState>((set, get) => ({
    ...load(),

    setSession: ({accessToken, refreshToken, backendUrl, instance, isResolver, resolverUrl}) => {
        set({accessToken, refreshToken, backendUrl, instance, isResolver, resolverUrl})
        persist(get())
    },
    setTokens: ({accessToken, refreshToken}) => {
        set({accessToken, refreshToken})
        persist(get())
    },
    setUser: (user) => {
        set({user})
        persist(get())
    },
    clear: () => {
        set(EMPTY)
        localStorage.removeItem(LS_KEY)
    },
}))
