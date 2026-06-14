import {originFor} from '@/lib/constants'

interface WebFingerJrd {
    subject?: string
    links?: Array<{ rel: string; href: string }>
}

/**
 * Resolve the backend base URL (scheme + host) hosting @username:instance via
 * WebFinger on `instance`. Throws a human-readable error on failure.
 */
export async function resolveBackendUrl(username: string, instance: string): Promise<string> {
    const resource = `archypix:@${username}:${instance}`
    const url = `${originFor(instance)}/.well-known/webfinger?resource=${encodeURIComponent(resource)}`

    let res: Response
    try {
        res = await fetch(url, {headers: {Accept: 'application/jrd+json'}})
    } catch {
        throw new Error(`Could not reach ${instance}. Check the instance domain and that it is online.`)
    }

    if (res.status === 404) {
        throw new Error(`No account @${username}:${instance} found on this instance.`)
    }
    if (!res.ok) {
        throw new Error(`WebFinger lookup on ${instance} failed (HTTP ${res.status}).`)
    }

    const jrd = (await res.json()) as WebFingerJrd
    const link = jrd.links?.find((l) => l.rel === 'backend_url')
    if (!link?.href) {
        throw new Error(`WebFinger response from ${instance} did not include a backend_url.`)
    }
    return link.href.replace(/\/+$/, '')
}
