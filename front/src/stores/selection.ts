import {create} from 'zustand'

/**
 * Multi-photo selection for the gallery. `selected` is the ordered list of
 * picture IDs. Range selection is computed against the grid's current visual
 * order, supplied by the caller at click time.
 */
interface SelectionState {
    selected: string[]
    /** Anchor for shift-range selection (last single/toggle click). */
    anchor: string | null
    /**
     * Touch multi-select mode. Entered via long-press (no modifier keys on mobile);
     * while on, a plain tap toggles a photo instead of replacing the selection.
     */
    multiSelect: boolean

    select: (id: string) => void
    toggle: (id: string) => void
    selectTo: (id: string, orderedIds: string[]) => void
    /** Start touch multi-select on a single photo (the long-pressed one). */
    enterMultiSelect: (id: string) => void
    setSelection: (ids: string[]) => void
    clear: () => void
}

export const useSelectionStore = create<SelectionState>((set, get) => ({
    selected: [],
    anchor: null,
    multiSelect: false,

    // A plain single select always exits multi-select mode.
    select: (id) => set({selected: [id], anchor: id, multiSelect: false}),

    toggle: (id) => {
        const {selected, multiSelect} = get()
        const next = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]
        // Deselecting the last photo leaves multi-select mode.
        set({selected: next, anchor: id, multiSelect: next.length === 0 ? false : multiSelect})
    },

    enterMultiSelect: (id) => set({selected: [id], anchor: id, multiSelect: true}),

    selectTo: (id, orderedIds) => {
        const {anchor} = get()
        if (!anchor) {
            set({selected: [id], anchor: id})
            return
        }
        const a = orderedIds.indexOf(anchor)
        const b = orderedIds.indexOf(id)
        if (a === -1 || b === -1) {
            set({selected: [id], anchor: id})
            return
        }
        const [lo, hi] = a < b ? [a, b] : [b, a]
        set({selected: orderedIds.slice(lo, hi + 1)})
    },

    setSelection: (ids) => set({selected: ids, anchor: ids[ids.length - 1] ?? null}),

    clear: () => set({selected: [], anchor: null, multiSelect: false}),
}))
