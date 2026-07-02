import {create} from 'zustand'
import type {PictureVariant} from '@/lib/types'

/**
 * Session registry of picture image URLs and which variants the browser has actually loaded.
 *
 * Presigned URLs already live in the TanStack Query cache, but this store additionally records the
 * *display* URL each `<img>` was given (including the grid's `thumbnail_url`, which never goes
 * through the `/url` query) together with whether that image finished loading. Consumers reuse the
 * best already-loaded variant so the carousel/lightbox/sidebar can paint a higher-res picture
 * instantly (from the browser cache, no new presign or download) before the final variant arrives.
 */

const RANK: Record<PictureVariant, number> = {small: 1, medium: 2, large: 3, original: 4}

export interface LoadedVariant {
    variant: PictureVariant
    url: string
}

interface Entry {
    url: string
    loaded: boolean
}

interface ImageCacheState {
    /** id → variant → { url, loaded }. */
    entries: Record<string, Partial<Record<PictureVariant, Entry>>>
    /** Record a URL assigned to a variant (before it has necessarily loaded). */
    record: (id: string, variant: PictureVariant, url: string, loaded?: boolean) => void
}

export const useImageCache = create<ImageCacheState>((set) => ({
    entries: {},
    record: (id, variant, url, loaded = false) =>
        set((s) => {
            const prev = s.entries[id]?.[variant]
            // No-op if nothing changed (avoids re-render churn from repeated onLoad).
            if (prev && prev.url === url && (prev.loaded || !loaded)) return s
            return {
                entries: {
                    ...s.entries,
                    [id]: {...s.entries[id], [variant]: {url, loaded: loaded || !!prev?.loaded}},
                },
            }
        }),
}))

/** Record a URL/loaded state imperatively (no subscription). */
export function recordImage(id: string, variant: PictureVariant, url: string | null | undefined, loaded = false): void {
    if (!url) return
    useImageCache.getState().record(id, variant, url, loaded)
}

/** Best already-loaded variant for a picture, optionally capped at `cap` (never larger than it). */
export function bestLoaded(
    entry: Partial<Record<PictureVariant, Entry>> | undefined,
    cap?: PictureVariant,
): LoadedVariant | null {
    if (!entry) return null
    const capRank = cap ? RANK[cap] : Infinity
    let best: LoadedVariant | null = null
    for (const v of Object.keys(entry) as PictureVariant[]) {
        const e = entry[v]
        if (!e?.loaded || RANK[v] > capRank) continue
        if (!best || RANK[v] > RANK[best.variant]) best = {variant: v, url: e.url}
    }
    return best
}

export {RANK as VARIANT_RANK}
