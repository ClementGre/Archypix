// The tag-metadata write queue (feature 34 §4.1).
//
// View preferences and ordering change far more often than they need persisting — a user nudging
// the grouping five times in half a minute must not produce five round trips. Every write goes
// through one queue that **coalesces per tag** (last value wins per field) and flushes on a 60 s
// trailing debounce, sending the whole coalesced batch as one `PUT /tags/meta`.

import {upsertTagMeta, upsertTagMetaKeepalive} from '@/api/tags'
import type {TagMetaPatch} from '@/lib/types'

const DEBOUNCE_MS = 60_000

type Listener = (paths: string[]) => void

const pending = new Map<string, TagMetaPatch>()
const listeners = new Set<Listener>()
let timer: ReturnType<typeof setTimeout> | undefined
let retried = false
/** Surfaced by the toast in `useTagMetaQueue` when a flush fails twice. */
let onError: ((message: string) => void) | undefined

/** Queue a partial write. Only the fields that actually changed should be passed, so a stale flush
 *  can never clobber a concurrent change from another device. */
export function queueTagMeta(patch: TagMetaPatch): void {
    const prev = pending.get(patch.tag_path)
    pending.set(patch.tag_path, prev ? {...prev, ...patch} : patch)
    for (const l of listeners) l([...pending.keys()])
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => void flushTagMeta(), DEBOUNCE_MS)
}

/** Whether a tag has an unflushed write (used to keep optimistic local state authoritative). */
export function pendingTagMeta(path: string): TagMetaPatch | undefined {
    return pending.get(path)
}

/** Every unflushed write. The tag payload is overlaid with these so a refetch — which the tree
 *  fires on most interactions — cannot revert a change the user just made (§4.1). */
export function allPendingTagMeta(): TagMetaPatch[] {
    return [...pending.values()]
}

export function onTagMetaQueueChange(listener: Listener): () => void {
    listeners.add(listener)
    return () => listeners.delete(listener)
}

export function setTagMetaQueueErrorHandler(handler: (message: string) => void): void {
    onError = handler
}

/**
 * Send the coalesced batch. `unload` switches to `fetch(keepalive)`, which survives document
 * teardown; the ordinary path goes through axios so the 401 → refresh → retry interceptor applies.
 *
 * A failed flush re-queues **once** and then surfaces a toast rather than silently discarding.
 */
export async function flushTagMeta(unload = false): Promise<void> {
    if (timer) {
        clearTimeout(timer)
        timer = undefined
    }
    if (pending.size === 0) return
    const items = [...pending.values()]
    pending.clear()
    for (const l of listeners) l([])
    try {
        if (unload) await upsertTagMetaKeepalive(items)
        else await upsertTagMeta(items)
        retried = false
    } catch (e) {
        if (retried) {
            retried = false
            onError?.(e instanceof Error ? e.message : 'Could not save tag preferences')
            return
        }
        retried = true
        for (const item of items) queueTagMeta(item)
    }
}

/** Flush now for a tag the user is navigating away from (§4.1) — the one write whose loss is
 *  visible is a reorder, which is why it goes through the same queue. */
export function flushTagMetaFor(path: string): void {
    if (pending.has(path)) void flushTagMeta()
}

/**
 * Install the immediate-flush triggers. `visibilitychange → hidden` covers tab switch and mobile
 * backgrounding, which on iOS is often the last event before the page is discarded.
 */
export function installTagMetaFlushHandlers(): () => void {
    const onHide = () => {
        if (document.visibilityState === 'hidden') void flushTagMeta(true)
    }
    const onPageHide = () => void flushTagMeta(true)
    document.addEventListener('visibilitychange', onHide)
    window.addEventListener('pagehide', onPageHide)
    window.addEventListener('beforeunload', onPageHide)
    return () => {
        document.removeEventListener('visibilitychange', onHide)
        window.removeEventListener('pagehide', onPageHide)
        window.removeEventListener('beforeunload', onPageHide)
    }
}
