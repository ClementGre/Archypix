import {create} from 'zustand'
import type {PictureFilter, PictureSelection} from '@/lib/types'

/**
 * Gallery multi-selection as the feature-14 **selection descriptor** (§2): a `query`
 * (the adopted view filter for select-all) plus explicit `includeIds` / `excludeIds`
 * deltas. The effective set is `(resolve(query) ∪ includeIds) \ excludeIds`.
 *
 * Two modes fall out of one model:
 * - **Explicit** — `query: null`, `includeIds` = the clicked pictures (the degenerate
 *   single-picture case is `includeIds.length === 1`).
 * - **Select-all** — `query` = the current view's filter, `excludeIds` = un-checked
 *   pictures, `includeIds` = pictures checked outside the query.
 *
 * Range selection is materialised into `includeIds` against the grid's visual order,
 * supplied by the caller at click time (§2.2).
 */
interface SelectionState {
    /** Adopted view filter for select-all mode; `null` ⇒ pure explicit selection. */
    query: PictureFilter | null
    /** Explicitly added picture ids (the only members when `query` is null). */
    includeIds: string[]
    /** Pictures subtracted from the query result (only meaningful when `query` is set). */
    excludeIds: string[]
    /** Anchor for shift-range selection (last single/toggle click). */
    anchor: string | null
    /**
     * Touch multi-select mode. Entered via long-press (no modifier keys on mobile);
     * while on, a plain tap toggles a photo instead of replacing the selection.
     */
    multiSelect: boolean
    /**
     * A deferred "land here" intent for the fix tools (feature 30 §8). Apply-and-next / Skip restore
     * the pre-reference view (the user may have navigated away to find references), so the landing
     * picture must be resolved **against the restored grid once it is showing**, not the reference
     * grid. `destSig` is the selection-filter signature of that destination view (captured when
     * reference picking began); `PhotoGrid` only resolves the intent once the on-screen view matches
     * it — the restore navigation lands a render *after* the store flips out of reference mode, so
     * without this the intent would resolve against the still-visible reference grid and the following
     * view-change clear would wipe it. `null` `destSig` means no view restore is pending (a plain fix
     * apply). `advance: false` keeps `anchorId`; `true` selects the next still-missing picture after it.
     */
    pendingLand: { anchorId: string; advance: boolean; destSig: string | null } | null

    select: (id: string) => void
    toggle: (id: string) => void
    selectTo: (id: string, orderedIds: string[]) => void
    /** Start touch multi-select on a single photo (the long-pressed one). */
    enterMultiSelect: (id: string) => void
    /** Replace the selection with an explicit id list. */
    setSelection: (ids: string[]) => void
    /** Queue a deferred land intent (resolved once the destination view is back on screen). */
    queueLand: (land: { anchorId: string; advance: boolean; destSig: string | null } | null) => void
    /** Adopt a query as a select-all (Ctrl+A) — clears the explicit deltas. */
    selectAll: (query: PictureFilter) => void
    /** Swap include/exclude relative to the query (only meaningful in select-all). */
    invert: (query: PictureFilter) => void
    clear: () => void
}

export const useSelectionStore = create<SelectionState>((set, get) => ({
    query: null,
    includeIds: [],
    excludeIds: [],
    anchor: null,
    multiSelect: false,
    pendingLand: null,

    // A plain single select always exits multi-select / select-all mode.
    select: (id) =>
        set({query: null, includeIds: [id], excludeIds: [], anchor: id, multiSelect: false}),

    queueLand: (land) => set({pendingLand: land}),

    toggle: (id) => {
        const {query, includeIds, excludeIds, multiSelect} = get()
        if (query) {
            // Select-all mode: a visible card is a query member, so toggling flips its
            // membership through the exclude delta (and drops any stale include entry).
            const next = excludeIds.includes(id)
                ? {excludeIds: excludeIds.filter((x) => x !== id)}
                : {excludeIds: [...excludeIds, id], includeIds: includeIds.filter((x) => x !== id)}
            set({...next, anchor: id})
            return
        }
        const nextInclude = includeIds.includes(id)
            ? includeIds.filter((x) => x !== id)
            : [...includeIds, id]
        // Deselecting the last photo leaves multi-select mode.
        set({includeIds: nextInclude, anchor: id, multiSelect: nextInclude.length === 0 ? false : multiSelect})
    },

    enterMultiSelect: (id) =>
        set({query: null, includeIds: [id], excludeIds: [], anchor: id, multiSelect: true}),

    selectTo: (id, orderedIds) => {
        const {anchor} = get()
        if (!anchor) {
            set({query: null, includeIds: [id], excludeIds: [], anchor: id})
            return
        }
        const a = orderedIds.indexOf(anchor)
        const b = orderedIds.indexOf(id)
        if (a === -1 || b === -1) {
            set({query: null, includeIds: [id], excludeIds: [], anchor: id})
            return
        }
        const [lo, hi] = a < b ? [a, b] : [b, a]
        set({query: null, includeIds: orderedIds.slice(lo, hi + 1), excludeIds: []})
    },

    setSelection: (ids) =>
        set({query: null, includeIds: ids, excludeIds: [], anchor: ids[ids.length - 1] ?? null}),

    selectAll: (query) => set({query, includeIds: [], excludeIds: [], anchor: null}),

    invert: (view_query) => {
        const {query, includeIds, excludeIds} = get()
        let new_query = query ? null : view_query;
        set({includeIds: excludeIds, excludeIds: includeIds, query: new_query})
    },

    clear: () => set({query: null, includeIds: [], excludeIds: [], anchor: null, multiSelect: false, pendingLand: null}),
}))

// ── Derived helpers ─────────────────────────────────────────────────────────

/** Whether anything is selected. */
export function hasSelection(s: SelectionState): boolean {
    return s.query !== null || s.includeIds.length > 0
}

/** A single explicit picture is selected (drives the single-picture detail panel). */
export function isSingleSelection(s: SelectionState): boolean {
    return s.query === null && s.includeIds.length === 1 && s.excludeIds.length === 0
}

/**
 * Membership of a grid card. In select-all mode a visible card is assumed to be a query
 * member (the grid renders the same view), so it is selected unless explicitly excluded.
 */
export function isMemberSelected(
    query: PictureFilter | null,
    includeIds: string[],
    excludeIds: string[],
    id: string,
): boolean {
    if (!query) return includeIds.includes(id)
    return includeIds.includes(id) || !excludeIds.includes(id)
}

/** Build the API selection descriptor from the store state, omitting empty deltas. */
export function toApiSelection(s: Pick<SelectionState, 'query' | 'includeIds' | 'excludeIds'>): PictureSelection {
    const sel: PictureSelection = {}
    if (s.query) sel.query = s.query
    if (s.includeIds.length) sel.include_ids = s.includeIds
    if (s.excludeIds.length) sel.exclude_ids = s.excludeIds
    return sel
}
