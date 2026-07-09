import axios, {type AxiosInstance} from 'axios'
import {createContext, useContext} from 'react'
import {apiClient} from '@/api/client'
import {installResolverAuth} from '@/api/resolverAdmin'

/**
 * The admin surface (`/api/admin/*`) is reached two ways (feature 24 §5):
 *  - **direct** — the logged-in user's `apiClient` (own instance, `/admin`);
 *  - **proxied** — through the resolver's delegation-replay proxy for a chosen backend (fleet
 *    drill-down), rewriting `/api/admin/foo` → `/api/resolver-admin/instances/{d}/api/admin/foo`.
 *
 * Admin api functions take an `AxiosInstance`; admin hooks read it (plus a cache `scope`) from this
 * context, so the *same* components render against either transport with no changes.
 */
export interface AdminClientCtx {
    client: AxiosInstance
    /** Cache-key scope so direct vs. per-backend queries never collide. `'local'` or a back_domain. */
    scope: string
}

const AdminClientContext = createContext<AdminClientCtx>({client: apiClient, scope: 'local'})

export const AdminClientProvider = AdminClientContext.Provider

export function useAdminClient(): AdminClientCtx {
    return useContext(AdminClientContext)
}

const proxyClients = new Map<string, AxiosInstance>()

/** A cached axios instance that proxies `/api/admin/*` to `backDomain` via the resolver. */
export function proxyAdminClient(backDomain: string): AxiosInstance {
    const existing = proxyClients.get(backDomain)
    if (existing) return existing
    // `installResolverAuth` sets `baseURL` to the connected resolver per request (feature 25).
    const instance = installResolverAuth(axios.create())
    instance.interceptors.request.use((config) => {
        if (config.url?.startsWith('/api/admin/')) {
            const sub = config.url.slice('/api/admin/'.length)
            config.url = `/api/resolver-admin/instances/${encodeURIComponent(backDomain)}/api/admin/${sub}`
        }
        return config
    })
    proxyClients.set(backDomain, instance)
    return instance
}
