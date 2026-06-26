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
