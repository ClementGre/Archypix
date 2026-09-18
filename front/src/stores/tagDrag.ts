import {create} from 'zustand'
import type {PictureSelection} from '@/lib/types'

/**
 * Drag-and-drop tagging (feature 34 §9). Desktop only: HTML5 `draggable` does not fire on touch, so
 * it cannot collide with the `onLongPress` multi-select in `PhotoGrid` — and the batch panel already
 * covers mobile.
 */
interface TagDragState {
    /** The dragged selection descriptor, or `null` when no drag is in flight. */
    selection: PictureSelection | null
    /** How many pictures are being dragged (the ghost's count badge). */
    count: number
    /**
     * The tag the drag started *inside*, when it started from a subtag block in Subtag view. Only
     * then is "sibling" defined — from the flat grid there is no source tag (§9).
     */
    sourceTag: string | null
    start: (drag: { selection: PictureSelection; count: number; sourceTag: string | null }) => void
    end: () => void
}

export const useTagDragStore = create<TagDragState>((set) => ({
    selection: null,
    count: 0,
    sourceTag: null,
    start: ({selection, count, sourceTag}) => set({selection, count, sourceTag}),
    end: () => set({selection: null, count: 0, sourceTag: null}),
}))

/** Siblings share a parent — the case where the drop dialog offers "add and remove". */
export function areSiblings(a: string, b: string): boolean {
    const parent = (p: string) => p.slice(0, Math.max(0, p.lastIndexOf('.')))
    return a !== b && parent(a) === parent(b)
}
