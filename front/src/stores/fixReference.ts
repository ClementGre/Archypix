import {create} from 'zustand'
import type {FixMode} from '@/lib/types'

/**
 * The two-step selection model of the fix tools (feature 30 §7). Pressing **Pick references**
 * stashes the current target ids and puts the gallery into a distinct **reference-picking phase**:
 * a fresh selection that **persists across tag navigation** (the defining behaviour of the phase),
 * used to derive a value (copy / interpolate / average) applied back over the stashed targets.
 *
 * Kept separate from the normal `selection` store so normal selection stays cleared-on-tag-change
 * and the reference set survives filter changes only while the phase is active.
 */
interface FixReferenceState {
    /** Reference-picking phase on. */
    active: boolean
    /** Which field the references derive (`gps` | `date`). */
    field: FixMode | null
    /** The stashed target picture ids the derived value applies to. */
    targetIds: string[]
    /** The chosen reference picture ids (persist across tag navigation while the phase is on). */
    refIds: string[]
    /** URL search string captured on entry, restored on exit (the phase lets the user freely
     * change tag/sort filters, then puts the gallery back the way it was). */
    savedSearch: string | null
    /** Selection-filter signature of the entry (destination) view, so the deferred land intent only
     * resolves once that view is showing again after the restore navigation (see `pendingLand`). */
    entrySig: string | null

    /** Enter the phase for a field, stashing the target ids + the URL/signature to restore on exit. */
    enter: (field: FixMode, targetIds: string[], savedSearch: string, entrySig: string) => void
    /** Toggle a reference in/out of the set. */
    toggleRef: (id: string) => void
    clearRefs: () => void
    /** Leave the phase, discarding the stashed targets and references (§12.7). */
    cancel: () => void
}

export const useFixReference = create<FixReferenceState>((set, get) => ({
    active: false,
    field: null,
    targetIds: [],
    refIds: [],
    savedSearch: null,
    entrySig: null,

    enter: (field, targetIds, savedSearch, entrySig) =>
        set({active: true, field, targetIds, refIds: [], savedSearch, entrySig}),

    toggleRef: (id) => {
        const {refIds} = get()
        set({refIds: refIds.includes(id) ? refIds.filter((x) => x !== id) : [...refIds, id]})
    },

    clearRefs: () => set({refIds: []}),

    cancel: () => set({active: false, field: null, targetIds: [], refIds: [], savedSearch: null, entrySig: null}),
}))
