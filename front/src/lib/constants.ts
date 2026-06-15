// Frontend configuration, derived from Vite env vars (see .env.example).
// The app is "federated": it talks directly to the backend resolved for the
// logged-in user via WebFinger, rather than a single fixed API base URL.

/** Default global/identity domain — the part after ':' in @user:domain handles. */
export const GLOBAL_DOMAIN = import.meta.env.VITE_GLOBAL_DOMAIN || 'archypix.test'

/** Whether to reach the global domain (and resolved backends) over HTTPS. */
export const USE_HTTPS = (import.meta.env.VITE_USE_HTTPS || 'false') === 'true'

export const SCHEME = USE_HTTPS ? 'https' : 'http'

export type RegistrationMode = 'auto' | 'resolver' | 'standalone'

/** Strategy for resolving the registration endpoint (see register() in api/auth.ts). */
export const REGISTRATION_MODE = (import.meta.env.VITE_REGISTRATION_MODE || 'auto') as RegistrationMode

/** Explicit registration endpoint override; when set, used verbatim. */
export const REGISTRATION_URL = import.meta.env.VITE_REGISTRATION_URL || ''

/** Build a scheme://host origin for a bare domain using the configured scheme. */
export function originFor(domain: string): string {
    return `${SCHEME}://${domain}`
}

/** Centralised TanStack Query keys, following the ['domain','list'|'detail',...] pattern. */
export const queryKeys = {
    pictures: (filters?: unknown) => ['pictures', filters] as const,
    picture: (id: string) => ['pictures', 'detail', id] as const,
    pictureJobs: (id: string) => ['pictures', 'jobs', id] as const,
    tags: () => ['tags', 'list'] as const,
    pictureTags: (id: string) => ['tags', 'detail', id] as const,
    taggingServices: () => ['tagging', 'list'] as const,
    taggingService: (id: string) => ['tagging', 'detail', id] as const,
    outgoingShares: () => ['shares', 'outgoing'] as const,
    incomingShares: () => ['shares', 'incoming'] as const,
    settings: () => ['settings'] as const,
    // admin
    adminInstanceHealth: () => ['admin', 'instance'] as const,
    adminStats: () => ['admin', 'stats'] as const,
    adminConsistency: () => ['admin', 'consistency'] as const,
    adminUsers: () => ['admin', 'users'] as const,
    adminUserStats: (id: string) => ['admin', 'users', id, 'stats'] as const,
    adminUserShares: (id: string) => ['admin', 'users', id, 'shares'] as const,
    adminJobs: (params?: unknown) => ['admin', 'jobs', params] as const,
    adminStaleJobs: () => ['admin', 'jobs', 'stale'] as const,
    adminErroredShares: () => ['admin', 'shares', 'errored'] as const,
    adminFederationInstances: () => ['admin', 'federation', 'instances'] as const,
}
