import {create} from 'zustand'

/** The last-used apply action of the fix tools (feature 30 §8), remembered as the split-button label. */
export type FixApplyMode = 'apply' | 'applyNext'

const LS_KEY = 'archypix_fix_prefs'

function load(): FixApplyMode {
    try {
        const raw = localStorage.getItem(LS_KEY)
        if (raw === 'apply' || raw === 'applyNext') return raw
    } catch {
        // ignore
    }
    return 'applyNext'
}

interface FixPrefsState {
    applyMode: FixApplyMode
    setApplyMode: (mode: FixApplyMode) => void
}

export const useFixPrefs = create<FixPrefsState>((set) => ({
    applyMode: load(),
    setApplyMode: (applyMode) => {
        try {
            localStorage.setItem(LS_KEY, applyMode)
        } catch {
            // ignore
        }
        set({applyMode})
    },
}))
