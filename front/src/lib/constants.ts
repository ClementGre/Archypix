// Frontend configuration, derived from Vite env vars (see .env.example).
// The app is "federated": it talks directly to the backend resolved for the
// logged-in user via /archypix-resolver/{info,resolve}, not a single fixed API base URL.

// window.__ENV__ (see public/env.js) is populated at container startup, taking
// precedence over import.meta.env which is baked in at build time.
function readEnv(key: 'VITE_GLOBAL_DOMAIN' | 'VITE_USE_HTTPS'): string | undefined {
    return window.__ENV__?.[key] || import.meta.env[key]
}

/** Default global/identity domain — the part after ':' in @user:domain handles. */
export const GLOBAL_DOMAIN = readEnv('VITE_GLOBAL_DOMAIN') || 'archypix.test'

/** Whether to reach the global domain (and resolved backends) over HTTPS. */
export const USE_HTTPS = (readEnv('VITE_USE_HTTPS') || 'false').toLocaleLowerCase() === 'true'

export const SCHEME = USE_HTTPS ? 'https' : 'http'

/** Build a scheme://host origin for a bare domain using the configured scheme. */
export function originFor(domain: string): string {
    return `${SCHEME}://${domain}`
}

/** Last instance the user typed on login/register, persisted so both pages stay in sync. */
const INSTANCE_LS_KEY = 'archypix_instance'

export function getPreferredInstance(): string {
    try {
        return localStorage.getItem(INSTANCE_LS_KEY) || GLOBAL_DOMAIN
    } catch {
        return GLOBAL_DOMAIN
    }
}

export function setPreferredInstance(domain: string): void {
    try {
        // Only persist a real override; the default global domain needs no storage.
        if (domain && domain !== GLOBAL_DOMAIN) localStorage.setItem(INSTANCE_LS_KEY, domain)
        else localStorage.removeItem(INSTANCE_LS_KEY)
    } catch {
        // localStorage may be unavailable (private mode); ignore.
    }
}

/** Centralised TanStack Query keys, following the ['domain','list'|'detail',...] pattern. */
export const queryKeys = {
    pictures: (filters?: unknown) => ['pictures', filters] as const,
    picture: (id: string) => ['pictures', 'detail', id] as const,
    pictureJobs: (id: string) => ['pictures', 'jobs', id] as const,
    aggregate: (selection: unknown, sections: unknown, provenance: unknown) =>
        ['pictures', 'aggregate', selection, sections, provenance] as const,
    tags: () => ['tags', 'list'] as const,
    pictureTags: (id: string) => ['tags', 'detail', id] as const,
    taggingServices: () => ['tagging', 'list'] as const,
    taggingService: (id: string) => ['tagging', 'detail', id] as const,
    outgoingShares: () => ['shares', 'outgoing'] as const,
    incomingShares: () => ['shares', 'incoming'] as const,
    hierarchies: () => ['hierarchies', 'list'] as const,
    hierarchy: (id: string) => ['hierarchies', 'detail', id] as const,
    hierarchyTree: (id: string, path: string) => ['hierarchies', 'tree', id, path] as const,
    hierarchyBrowse: (id: string, path: string, filters?: unknown) =>
        ['hierarchies', 'browse', id, path, filters] as const,
    hierarchyWebdav: (id: string) => ['hierarchies', 'webdav', id] as const,
    settings: () => ['settings'] as const,
    storage: () => ['storage'] as const,
    invites: () => ['invites'] as const,
    invitations: () => ['invitations'] as const,
    // admin (backend runtime config, feature 23/24)
    adminSettings: () => ['admin', 'settings'] as const,
    adminRoutines: () => ['admin', 'routines'] as const,
    adminInvites: () => ['admin', 'invites'] as const,
    adminInstanceHealth: () => ['admin', 'instance'] as const,
    adminStats: () => ['admin', 'stats'] as const,
    adminConsistency: () => ['admin', 'consistency'] as const,
    adminUsers: () => ['admin', 'users'] as const,
    adminUserStats: (id: string) => ['admin', 'users', id, 'stats'] as const,
    adminUserShares: (id: string) => ['admin', 'users', id, 'shares'] as const,
    adminUserStorageAudit: (id: string) => ['admin', 'users', id, 'storage-audit'] as const,
    adminJobs: (params?: unknown) => ['admin', 'jobs', params] as const,
    adminStaleJobs: () => ['admin', 'jobs', 'stale'] as const,
    adminErroredShares: () => ['admin', 'shares', 'errored'] as const,
    adminFederationInstances: () => ['admin', 'federation', 'instances'] as const,
    // resolver fleet admin (feature 24)
    resolverAdmin: {
        overview: () => ['resolverAdmin', 'overview'] as const,
        backends: () => ['resolverAdmin', 'backends'] as const,
        settings: () => ['resolverAdmin', 'settings'] as const,
        routines: () => ['resolverAdmin', 'routines'] as const,
        invites: () => ['resolverAdmin', 'invites'] as const,
        configMatrix: () => ['resolverAdmin', 'config-matrix'] as const,
        proxy: (backDomain: string, path: string) => ['resolverAdmin', 'proxy', backDomain, path] as const,
    },
}
