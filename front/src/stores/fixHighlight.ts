import {create} from 'zustand'

/**
 * The GPS-fix before/after interpolation anchor ids (feature 30 §5.2), published by `GpsFixPanel`
 * so `PhotoGrid` can highlight those two source pictures in the grid. Session-only.
 */
interface FixHighlightState {
    anchorIds: string[]
    setAnchorIds: (ids: string[]) => void
}

export const useFixHighlight = create<FixHighlightState>((set) => ({
    anchorIds: [],
    setAnchorIds: (ids) => set({anchorIds: ids}),
}))
