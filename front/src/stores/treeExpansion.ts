import {create} from 'zustand'

const SS_KEY = 'archypix_tree_expansion'

type Trees = Record<string, string[]>

function load(): Trees {
    try {
        const raw = sessionStorage.getItem(SS_KEY)
        if (raw) return JSON.parse(raw) as Trees
    } catch {
        // ignore malformed storage
    }
    return {}
}

function persist(trees: Trees) {
    try {
        sessionStorage.setItem(SS_KEY, JSON.stringify(trees))
    } catch {
        // ignore quota / privacy-mode failures
    }
}

/**
 * Which rows of the sidebar trees are open, per tree (`tags`, `hierarchy:<id>`). Kept for the
 * browser tab's session, so leaving the gallery and coming back keeps the trees as they were.
 */
interface TreeExpansionState {
    trees: Trees
    toggle: (tree: string, key: string) => void
    expand: (tree: string, keys: Iterable<string>) => void
}

export const useTreeExpansion = create<TreeExpansionState>((set) => {
    const write = (fn: (trees: Trees) => Trees) =>
        set((s) => {
            const trees = fn(s.trees)
            persist(trees)
            return {trees}
        })
    return {
        trees: load(),
        toggle: (tree, key) =>
            write((t) => {
                const open = t[tree] ?? []
                return {...t, [tree]: open.includes(key) ? open.filter((k) => k !== key) : [...open, key]}
            }),
        expand: (tree, keys) =>
            write((t) => {
                const open = new Set(t[tree] ?? [])
                const before = open.size
                for (const k of keys) open.add(k)
                return open.size === before ? t : {...t, [tree]: [...open]}
            }),
    }
})

const EMPTY: string[] = []

/** The open rows of `tree`. */
export function useExpanded(tree: string): string[] {
    return useTreeExpansion((s) => s.trees[tree] ?? EMPTY)
}
