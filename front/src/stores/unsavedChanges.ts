import {useEffect, useRef} from 'react'
import {create} from 'zustand'

/**
 * Unsaved-changes guard shared by the editors (hierarchy, tagging service). An editor with
 * changes registers its save; a navigation that would close it goes through `guard`, which holds
 * it until the user saves, discards or cancels in `UnsavedChangesDialog`.
 */
interface UnsavedChangesState {
    /** Which editor has unsaved changes (e.g. `hierarchy:<id>`), or null. */
    owner: string | null
    save: (() => Promise<boolean>) | null
    pending: (() => void) | null
    register: (owner: string | null, save: (() => Promise<boolean>) | null) => void
    /** Run `go` now, or once the open editor's changes are saved or discarded. */
    guard: (go: () => void) => void
    settle: (choice: 'save' | 'discard' | 'cancel') => Promise<void>
}

export const useUnsavedChanges = create<UnsavedChangesState>((set, get) => ({
    owner: null,
    save: null,
    pending: null,
    register: (owner, save) => set({owner, save}),
    guard: (go) => (get().save ? set({pending: go}) : go()),
    settle: async (choice) => {
        const {save, pending} = get()
        if (choice === 'cancel') return set({pending: null})
        if (choice === 'save' && save && !(await save())) return set({pending: null})
        set({pending: null, owner: null, save: null})
        pending?.()
    },
}))

/** Register an editor's unsaved changes while `dirty`; also warns before the tab is closed. */
export function useRegisterUnsaved(owner: string, dirty: boolean, save: () => Promise<boolean>) {
    const saveRef = useRef(save)
    useEffect(() => {
        saveRef.current = save
    })
    useEffect(() => {
        const register = useUnsavedChanges.getState().register
        if (!dirty) return
        register(owner, () => saveRef.current())
        const warn = (e: BeforeUnloadEvent) => e.preventDefault()
        window.addEventListener('beforeunload', warn)
        return () => {
            register(null, null)
            window.removeEventListener('beforeunload', warn)
        }
    }, [owner, dirty])
}
