import {originFor} from '@/lib/constants'

/** Bootstrap discovery shape returned by `GET /archypix-resolver/info` (feature 25). */
export interface ResolverInfo {
    is_resolver: boolean
    /** Base URL for the heavier surface: `${api_url}/api/public/register`, `/api/resolver-admin/...`. */
    api_url: string
}

export interface ResolvedConnection {
    /** Owning backend base URL (scheme + host) to talk to for the authenticated API. */
    backendUrl: string
    /** Whether the queried domain is fronted by a resolver (⇒ a fleet dashboard exists). */
    isResolver: boolean
    /** Resolver `api_url` (`…/archypix-resolver`) when `isResolver`, else `null`. */
    resolverUrl: string | null
}

function normalizeBase(url: string): string {
    return url.trim().replace(/\/+$/, '')
}

/**
 * Bootstrap a domain: `GET {domain}/archypix-resolver/info`. Answered directly at the domain by
 * whatever sits there (a standalone backend ⇒ `is_resolver:false`; a resolver ⇒ `is_resolver:true`).
 * Throws a human-readable error on failure.
 */
export async function getResolverInfo(domain: string): Promise<ResolverInfo> {
    let res: Response
    try {
        res = await fetch(`${originFor(domain)}/archypix-resolver/info`, {headers: {Accept: 'application/json'}})
    } catch {
        throw new Error(`Could not reach ${domain}. Check the domain and that it is online.`)
    }
    if (!res.ok) {
        throw new Error(`Discovery on ${domain} failed (HTTP ${res.status}).`)
    }
    const info = (await res.json()) as ResolverInfo
    return {is_resolver: !!info.is_resolver, api_url: normalizeBase(info.api_url)}
}

/**
 * Resolve `@username:instance` to the backend it should talk to, and learn whether the instance is
 * resolver-fronted. One bootstrap call (`/info`); when a resolver exists, one more direct
 * `/archypix-resolver/resolve` call at the instance (replaces the old WebFinger lookup).
 */
export async function resolveConnection(username: string, instance: string): Promise<ResolvedConnection> {
    const info = await getResolverInfo(instance)
    if (!info.is_resolver) {
        // Standalone backend: nothing to resolve, `api_url` is the backend itself.
        return {backendUrl: info.api_url, isResolver: false, resolverUrl: null}
    }
    const backendUrl = await resolveViaResolver(username, instance)
    return {backendUrl, isResolver: true, resolverUrl: info.api_url}
}

/** Hit the resolver's `/archypix-resolver/resolve` directly at `instance`. */
async function resolveViaResolver(username: string, instance: string): Promise<string> {
    const params = new URLSearchParams({user: username, domain: instance})
    const url = `${originFor(instance)}/archypix-resolver/resolve?${params.toString()}`
    let res: Response
    try {
        res = await fetch(url, {headers: {Accept: 'application/json'}})
    } catch {
        throw new Error(`Could not reach ${instance}. Check the domain and that it is online.`)
    }
    if (res.status === 404) {
        throw new Error(`No account @${username}:${instance} found on this instance.`)
    }
    if (!res.ok) {
        throw new Error(`Resolving @${username}:${instance} failed (HTTP ${res.status}).`)
    }
    const body = (await res.json()) as { backend_url?: string }
    if (!body.backend_url) {
        throw new Error(`Resolver on ${instance} did not return a backend_url.`)
    }
    return normalizeBase(body.backend_url)
}
