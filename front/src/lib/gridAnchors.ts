// Shared GPS interpolation-anchor resolution for the fix tools (feature 30 §5.2). Both the
// single-target panel (useFixAnchors) and the bulk preview (fixBulk) pick their before/after anchors
// the same way, so single and bulk apply always agree.

import type {PictureListItem} from '@/lib/types'

export interface GridNeighbours {
    before: PictureListItem | null
    after: PictureListItem | null
}

const usableAnchor = (it: PictureListItem, targetId: string) =>
    it.id !== targetId && it.has_gps && !!it.captured_at

/** Assign two candidates to before/after by capture time — earlier is "before"; a tie keeps `a` first. */
const byTime = (a: PictureListItem, b: PictureListItem): GridNeighbours =>
    a.captured_at! <= b.captured_at! ? {before: a, after: b} : {before: b, after: a}

/**
 * The GPS-bearing pictures immediately on each side of the target **in grid order**.
 *
 * Grid adjacency — not a global time min/max — decides the anchors, so:
 *  - among several photos sharing the target's timestamp, the one physically closest in the grid wins;
 *  - a same-date photo sitting on each side of the target contributes one anchor to *each* slot.
 * before/after are then labelled by the anchors' own capture time (the grid may be sorted either way —
 * the default is `captured_at` descending), so a same-date pair keeps grid order (nearer side first).
 * When only one grid side has a GPS neighbour, its slot is chosen by its time vs the target and the
 * other side is left null for the caller to fill (the single panel's directed `captured_before/after`
 * lookup).
 *
 * If the target isn't loaded in the grid at all (a bulk selection can reach off-page pictures), it
 * falls back to the nearest-in-time GPS pictures over the loaded grid.
 */
export function gridGpsNeighbours(items: PictureListItem[], targetId: string, capturedAt: string): GridNeighbours {
    const idx = items.findIndex((it) => it.id === targetId)
    if (idx !== -1) {
        let lower: PictureListItem | null = null
        for (let i = idx - 1; i >= 0; i--) if (usableAnchor(items[i], targetId)) {
            lower = items[i];
            break
        }
        let upper: PictureListItem | null = null
        for (let i = idx + 1; i < items.length; i++) if (usableAnchor(items[i], targetId)) {
            upper = items[i];
            break
        }

        if (lower && upper) return byTime(lower, upper)
        const only = lower ?? upper
        if (!only) return {before: null, after: null}
        // A same-instant single neighbour counts as the "after" (weighted fully), matching the backend's
        // inclusive `captured_after >=` bracketing.
        return only.captured_at! < capturedAt ? {before: only, after: null} : {before: null, after: only}
    }

    // Off-page target: nearest-in-time GPS pictures over whatever grid is loaded.
    let before: PictureListItem | null = null
    let after: PictureListItem | null = null
    for (const it of items) {
        if (!usableAnchor(it, targetId)) continue
        if (it.captured_at! < capturedAt) {
            if (!before || it.captured_at! > before.captured_at!) before = it
        } else {
            if (!after || it.captured_at! < after.captured_at!) after = it
        }
    }
    return {before, after}
}
