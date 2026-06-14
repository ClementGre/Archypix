import {create} from 'zustand'

export type Theme = 'dark' | 'light'

const LS_KEY = 'archypix_theme'

function read(): Theme {
    const stored = localStorage.getItem(LS_KEY)
    return stored === 'light' ? 'light' : 'dark'
}

/** Apply the theme by toggling the `.light` class (dark is the base @theme). */
function apply(theme: Theme) {
    const root = document.documentElement
    root.classList.toggle('light', theme === 'light')
    root.classList.remove('dark') // base theme is dark; the static class is redundant
    localStorage.setItem(LS_KEY, theme)
}

interface ThemeState {
    theme: Theme
    toggle: () => void
    set: (theme: Theme) => void
}

export const useThemeStore = create<ThemeState>((set, get) => ({
    theme: read(),
    toggle: () => {
        const next: Theme = get().theme === 'dark' ? 'light' : 'dark'
        apply(next)
        set({theme: next})
    },
    set: (theme) => {
        apply(theme)
        set({theme})
    },
}))

/** Apply the persisted theme on app boot, before first paint. */
export function initTheme() {
    apply(read())
}
