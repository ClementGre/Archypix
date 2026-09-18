import {create} from 'zustand'

/**
 * Which subtag blocks the user has opened or closed (feature 35 §10.11). Transient on purpose: five
 * expanded groups would bloat a link, and expansion is not a preference — it is neither URL state
 * nor `tag_metadata`.
 *
 * An entry is an explicit **override** of the auto-expansion default (§5), so closing a block that
 * auto-expanded sticks.
 */
interface TimelineExpansionState {
    overrides: Record<string, boolean>
    setExpanded: (path: string, expanded: boolean) => void
    collapseAll: () => void
}

export const useTimelineExpansion = create<TimelineExpansionState>((set) => ({
    overrides: {},
    setExpanded: (path, expanded) =>
        set((s) => ({overrides: {...s.overrides, [path]: expanded}})),
    collapseAll: () => set({overrides: {}}),
}))
