import {useEffect, useState} from 'react'
import {TriangleAlert} from 'lucide-react'
import {originFor} from '@/lib/constants'

type Health = 'checking' | 'ok' | 'cors' | 'down'

/**
 * Actively probes an instance (feature 25) instead of showing a blanket CORS caveat: it pings
 * `{instance}/archypix-resolver/info` and reports only a *real* problem — the instance is unreachable,
 * or it is reachable but rejects this app's origin (CORS). Renders nothing while checking or when the
 * instance is reachable with a valid CORS config.
 */
export function InstanceHealthWarning({instance}: { instance: string }) {
    const [health, setHealth] = useState<Health>('checking')

    useEffect(() => {
        const domain = instance?.trim()
        if (!domain) {
            setHealth('checking')
            return
        }
        setHealth('checking')
        let cancelled = false
        // Debounce so we don't probe on every keystroke while the domain is being typed.
        const t = setTimeout(async () => {
            const result = await probe(domain)
            if (!cancelled) setHealth(result)
        }, 600)
        return () => {
            cancelled = true
            clearTimeout(t)
        }
    }, [instance])

    if (health === 'checking' || health === 'ok') return null

    const message =
        health === 'cors'
            ? `Reached ${instance}, but it rejected this app's origin (CORS). You are not allowed to connect to this domain from this frontend.`
            : `Can’t reach ${instance}. Check the domain and that it is online.`
    const tone =
        health === 'cors'
            ? 'border-amber-500/40 bg-amber-500/10 text-amber-600 dark:text-amber-400'
            : 'border-destructive/40 bg-destructive/10 text-destructive'

    return (
        <div className={`flex items-start gap-2 rounded-md border px-3 py-2 text-xs ${tone}`}>
            <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
            <span>{message}</span>
        </div>
    )
}

/**
 * Classify an instance's reachability. A normal (CORS) fetch that *resolves* — even to a 404 — means
 * CORS is fine; a throw means CORS blocked or the host is down. A follow-up `no-cors` probe (opaque,
 * ignores CORS) distinguishes the two: it succeeds if the host is up but CORS-blocked, throws if down.
 */
async function probe(domain: string): Promise<Health> {
    const url = `${originFor(domain)}/archypix-resolver/info`
    try {
        await fetch(url, {headers: {Accept: 'application/json'}})
        return 'ok'
    } catch {
        // CORS-blocked or unreachable — disambiguate below.
    }
    try {
        await fetch(url, {mode: 'no-cors'})
        return 'cors'
    } catch {
        return 'down'
    }
}
