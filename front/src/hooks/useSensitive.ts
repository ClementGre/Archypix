import {createContext, useContext, useMemo} from 'react'
import {useAllTags} from '@/hooks/useTags'
import {useSensitiveStore} from '@/stores/sensitive'
import type {PresenceFilter, SensitiveDisplayMode} from '@/lib/types'

/** How one sensitive picture renders right now (feature 38 §3.1). */
export type Veil = 'visible' | 'blurred' | 'redacted'

/**
 * The base display mode. The authenticated shell provides the user's setting; public pages get the
 * default — visitors always see the owner's sensitive pictures blurred (§7.1).
 */
const SensitiveModeContext = createContext<SensitiveDisplayMode>('blurred')
export const SensitiveModeProvider = SensitiveModeContext.Provider

export function useSensitiveMode(): SensitiveDisplayMode {
    return useContext(SensitiveModeContext)
}

export function useShowAllActive(): boolean {
    return useSensitiveStore((s) => s.showAllUntil != null)
}

/** Render precedence (§3.1): individual reveal, then show-all, then the mode. */
export function useVeil(id: string, sensitive: boolean | null | undefined): Veil {
    const mode = useSensitiveMode()
    const revealed = useSensitiveStore((s) => s.revealed.has(id))
    const showAll = useShowAllActive()
    if (!sensitive || revealed || showAll || mode === 'visible') return 'visible'
    return mode === 'blurred' ? 'blurred' : 'redacted'
}

/** The entries of `flagged` that `path` is at or under (wire form). */
export function flaggedAncestors(path: string, flagged: readonly string[]): string[] {
    return flagged.filter((f) => path === f || path.startsWith(`${f}.`))
}

/** The caller's sensitive tags (wire form), from the app-start tag payload. */
export function useSensitiveTags(): string[] {
    const {data} = useAllTags()
    return useMemo(() => (data ?? []).filter((i) => i.meta?.sensitive).map((i) => i.path), [data])
}

/**
 * The `sensitive` list param for the effective mode (§5.1): `missing` only in Hidden mode, without
 * the show-all override, and outside a flagged tag scope (§3.2 — there they render Redacted).
 */
export function useSensitiveListFilter(tag: string | null): PresenceFilter | undefined {
    const mode = useSensitiveMode()
    const showAll = useShowAllActive()
    const flagged = useSensitiveTags()
    if (mode !== 'hidden' || showAll) return undefined
    if (tag && flaggedAncestors(tag, flagged).length) return undefined
    return 'missing'
}
