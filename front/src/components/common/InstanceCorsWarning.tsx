import {TriangleAlert} from 'lucide-react'
import {GLOBAL_DOMAIN} from '@/lib/constants'

/**
 * Warns that talking to a custom instance (login or register) only works if this
 * frontend's URL is in that backend's CORS allowlist. Renders nothing for the
 * default global domain.
 */
export function InstanceCorsWarning({instance}: { instance: string }) {
    if (!instance || instance === GLOBAL_DOMAIN) return null
    return (
        <div
            className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
            <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
            <span>
                Connecting to a custom instance only works if this frontend's URL is in that backend's CORS
                allowlist.
            </span>
        </div>
    )
}
