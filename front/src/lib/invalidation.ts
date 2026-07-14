// Centralised query invalidation.
//
// TanStack matches invalidation keys by **deep-partial prefix**, so the key must be a real prefix
// of the cached keys. Use the bare top-level arrays here, NOT the parametrised `queryKeys.*`
// builders: `queryKeys.pictures()` is `['pictures', undefined]` (matches *nothing*) and
// `queryKeys.tags()` is `['tags', 'list']` (misses the per-picture `['tags', 'detail', id]` caches).
//
// `['pictures']` covers every picture query: lists (`['pictures', filters, variant]`), details
// (`['pictures', 'detail', id]`), aggregates, jobs, and presign URLs. `['tags']` covers the tag
// list, per-picture tags, and provenance.

import type {QueryClient} from '@tanstack/react-query'

// The backend tagging pipeline coalesces events over a debounce window (`PIPELINE_DEBOUNCE_MS`,
// ~5s) before (re)assigning rule/segment tags. A single immediate invalidation therefore fires
// before those tags exist; a delayed second pass picks them up. (Best-effort — no server push.)
const PIPELINE_SETTLE_MS = 6000

/** Invalidate all picture caches (lists, details, aggregates, jobs, presign URLs). */
export function invalidatePictures(qc: QueryClient): void {
    void qc.invalidateQueries({queryKey: ['pictures']})
}

/**
 * Optimistically drop pictures from every cached grid list (flat + hierarchy browse) so a delete
 * disappears immediately — without waiting for a refetch, and regardless of which page/offset the
 * item sat on. A background refetch (via {@link invalidatePictures}) then reconciles totals. Only
 * touches infinite-list caches (shape `{ pages: [{ items, total }] }`); detail/url/aggregate
 * queries under `['pictures']`/`['hierarchies']` are left untouched.
 */
export function removePicturesFromLists(qc: QueryClient, ids: string[]): void {
    const drop = new Set(ids)
    type Page = { items: Array<{ id: string }>; total: number }
    type Infinite = { pages: Page[]; pageParams: unknown[] }
    const isInfinite = (d: unknown): d is Infinite =>
        !!d && typeof d === 'object' && Array.isArray((d as { pages?: unknown }).pages)
    // Only grid lists that DON'T show trashed items — in a view that shows the trash (`include` or
    // `only`) a just-trashed picture legitimately stays visible, so removing it there would flash it
    // out then back on refetch.
    const showsTrashed = (f: unknown) => {
        const t = !!f && typeof f === 'object' ? (f as { trash?: string }).trash : undefined
        return t === 'include' || t === 'only'
    }
    const predicate = ({queryKey: k}: { queryKey: readonly unknown[] }) => {
        if (k[0] === 'pictures') return typeof k[1] === 'object' && k[1] !== null && !showsTrashed(k[1])
        if (k[0] === 'hierarchies' && k[1] === 'browse') return !showsTrashed(k[4])
        return false
    }
    const update = (old: unknown) => {
        if (!isInfinite(old)) return old
        let removed = 0
        const pages = old.pages.map((p) => {
            const items = p.items.filter((it) => {
                const keep = !drop.has(it.id)
                if (!keep) removed++
                return keep
            })
            return {...p, items}
        })
        if (!removed) return old
        // `total` is the global count repeated on every page — drop it uniformly.
        return {...old, pages: pages.map((p) => ({...p, total: Math.max(0, (p.total ?? 0) - removed)}))}
    }
    qc.setQueriesData({predicate}, update)
}

/** Invalidate all tag caches (the tag list, per-picture tags, provenance). */
export function invalidateTags(qc: QueryClient): void {
    void qc.invalidateQueries({queryKey: ['tags']})
}

/**
 * Invalidate pictures + tags **now and again after the pipeline settle window**, so both the
 * synchronous change and any asynchronously-assigned rule/segment tags surface. Use for anything
 * that can trigger background re-tagging: uploads, manual tag edits, EXIF/metadata edits,
 * tagging-service edits, share accept.
 */
export function invalidatePicturesAndTags(qc: QueryClient): void {
    const run = () => {
        invalidatePictures(qc)
        invalidateTags(qc)
    }
    run()
    setTimeout(run, PIPELINE_SETTLE_MS)
}

const STORAGE_INVALIDATE_DEBOUNCE_MS = 6000
let storageInvalidateTimer: ReturnType<typeof setTimeout> | undefined

/**
 * Debounced storage-query invalidation (feature 22): trash/restore/upload can fire in bursts
 * (batch actions, multi-file uploads), so instead of one refetch per picture this coalesces them
 * into a single `['storage']` refetch 6s after the last call.
 */
export function invalidateStorageDebounced(qc: QueryClient): void {
    if (storageInvalidateTimer) clearTimeout(storageInvalidateTimer)
    storageInvalidateTimer = setTimeout(() => {
        storageInvalidateTimer = undefined
        void qc.invalidateQueries({queryKey: ['storage']})
    }, STORAGE_INVALIDATE_DEBOUNCE_MS)
}
