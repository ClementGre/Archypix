import {create} from 'zustand'

/** Show-all durations offered in the reveal menus (minutes). */
export const SHOW_ALL_PRESETS = [5, 10, 30, 60] as const

/**
 * Tab-scoped reveals of sensitive pictures (feature 38 §4). Never persisted: a reload or a new tab
 * re-veils everything.
 */
interface SensitiveState {
    /** Pictures revealed one by one. */
    revealed: ReadonlySet<string>
    /** Epoch ms the show-all override ends at; `null` when off. */
    showAllUntil: number | null
    setRevealed: (id: string, revealed: boolean) => void
    startShowAll: (minutes: number) => void
    stopShowAll: () => void
}

let expiryTimer: number | undefined

export const useSensitiveStore = create<SensitiveState>((set) => ({
    revealed: new Set(),
    showAllUntil: null,
    setRevealed: (id, revealed) =>
        set((s) => {
            const next = new Set(s.revealed)
            if (revealed) next.add(id)
            else next.delete(id)
            return {revealed: next}
        }),
    startShowAll: (minutes) => {
        window.clearTimeout(expiryTimer)
        expiryTimer = window.setTimeout(() => set({showAllUntil: null}), minutes * 60_000)
        set({showAllUntil: Date.now() + minutes * 60_000})
    },
    stopShowAll: () => {
        window.clearTimeout(expiryTimer)
        set({showAllUntil: null})
    },
}))
