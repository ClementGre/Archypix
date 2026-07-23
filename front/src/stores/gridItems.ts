import {create} from 'zustand'
import type {PictureListItem} from '@/lib/types'

/**
 * The pictures currently loaded in the centre grid, in visual (sorted) order. Published by
 * `PhotoGrid` so the fix panels can scan for grid-local GPS interpolation anchors (feature 30 §5.2)
 * without a round-trip. Session-only, never persisted.
 */
interface GridItemsState {
    items: PictureListItem[]
    setItems: (items: PictureListItem[]) => void
}

export const useGridItems = create<GridItemsState>((set) => ({
    items: [],
    setItems: (items) => set({items}),
}))
