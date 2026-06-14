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

    select: (id: string) => void
    toggle: (id: string) => void
    selectTo: (id: string, orderedIds: string[]) => void
    setSelection: (ids: string[]) => void
    clear: () => void
}

export const useSelectionStore = create<SelectionState>((set, get) => ({
    selected: [],
    anchor: null,

    select: (id) => set({selected: [id], anchor: id}),

    toggle: (id) => {
        const {selected} = get()
        const next = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]
        set({selected: next, anchor: id})
    },

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

    clear: () => set({selected: [], anchor: null}),
}))
