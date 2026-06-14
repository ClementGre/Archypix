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

    setSession: (s: { accessToken: string; refreshToken: string; backendUrl: string; instance: string }) => void
    setTokens: (t: { accessToken: string; refreshToken: string }) => void
    setUser: (user: AuthUser | null) => void
    clear: () => void
}

const LS_KEY = 'archypix_auth'

type Persisted = Pick<AuthState, 'user' | 'accessToken' | 'refreshToken' | 'backendUrl' | 'instance'>

const EMPTY: Persisted = {
    user: null,
    accessToken: null,
    refreshToken: null,
    backendUrl: null,
    instance: null,
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
    const {user, accessToken, refreshToken, backendUrl, instance} = state
    localStorage.setItem(LS_KEY, JSON.stringify({user, accessToken, refreshToken, backendUrl, instance}))
}

export const useAuthStore = create<AuthState>((set, get) => ({
    ...load(),

    setSession: ({accessToken, refreshToken, backendUrl, instance}) => {
        set({accessToken, refreshToken, backendUrl, instance})
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
