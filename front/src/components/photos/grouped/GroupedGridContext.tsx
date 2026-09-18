// The flat visible order across nested sections (feature 35 §8).
//
// Three consumers need one ordered array and all three are load-bearing: `orderedIds` → `selectTo`
// (shift-click), `useGridItems` (feature 30 §5.2 anchor scan), and the Lightbox's items + paging.
// With nested sections there is no longer one array, so each mounted section registers itself and
// the context exposes the flattened visible render order.
//
// Registration carries `fetchNextPage` because ordering alone is not enough: paging past the last
// item of section *k* has to advance **that section's** query, not a global one.
//
// The API and the data are two contexts on purpose: registering a section would otherwise re-render
// every other section, which is quadratic on a deep expand-all.

import {createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState} from 'react'
import type {PictureListItem} from '@/lib/types'

export interface SectionRegistration {
    /** Position in the tree, one entry per nesting level — compared lexicographically. */
    order: number[]
    items: PictureListItem[]
    fetchNextPage: () => void
    hasNextPage: boolean
}

interface GroupedGridApi {
    register: (key: string, reg: SectionRegistration) => void
    unregister: (key: string) => void
    /** Read the flattened order without subscribing to it — for click handlers. */
    getOrderedIds: () => string[]
}

interface GroupedGridData {
    items: PictureListItem[]
    /** Advance the section owning `anchorId` (the picture the viewer is on), else the next one. */
    loadMore: (anchorId?: string | null) => void
    hasMore: boolean
}

const ApiContext = createContext<GroupedGridApi | null>(null)
const DataContext = createContext<GroupedGridData>({items: [], loadMore: () => undefined, hasMore: false})

function compareOrder(a: number[], b: number[]): number {
    for (let i = 0; i < Math.max(a.length, b.length); i++) {
        const av = a[i] ?? -Infinity
        const bv = b[i] ?? -Infinity
        if (av !== bv) return av - bv
    }
    return 0
}

const byOrder = (entries: [string, SectionRegistration][]) =>
    [...entries].sort((a, b) => compareOrder(a[1].order, b[1].order))

export function GroupedGridProvider({children}: { children: ReactNode }) {
    const [sections, setSections] = useState<Map<string, SectionRegistration>>(new Map)

    const items = useMemo(() => {
        const seen = new Set<string>()
        const out: PictureListItem[] = []
        for (const [, reg] of byOrder([...sections.entries()])) {
            for (const it of reg.items) {
                // The same picture can sit in a parent's direct photos and a child's block (§10.12).
                if (seen.has(it.id)) continue
                seen.add(it.id)
                out.push(it)
            }
        }
        return out
    }, [sections])

    const orderedIds = useMemo(() => items.map((i) => i.id), [items])

    // `loadMore` and `getOrderedIds` read the live state without re-subscribing their callers, so
    // registering a section does not re-render every card list.
    const latest = useRef({sections, orderedIds})
    useEffect(() => {
        latest.current = {sections, orderedIds}
    }, [sections, orderedIds])

    const api = useMemo<GroupedGridApi>(
        () => ({
            register: (key, reg) =>
                setSections((prev) => {
                    const next = new Map(prev)
                    next.set(key, reg)
                    return next
                }),
            unregister: (key) =>
                setSections((prev) => {
                    if (!prev.has(key)) return prev
                    const next = new Map(prev)
                    next.delete(key)
                    return next
                }),
            getOrderedIds: () => latest.current.orderedIds,
        }),
        [],
    )

    // Ordering alone is not enough: paging past the last item of section *k* has to advance **that**
    // section's query. Advancing the first unfinished section instead would keep inserting rows
    // *before* the viewer and never reach the one they are in.
    const loadMore = useCallback((anchorId?: string | null) => {
        const list = byOrder([...latest.current.sections.entries()])
        const at = anchorId ? list.findIndex(([, r]) => r.items.some((i) => i.id === anchorId)) : -1
        for (const [, reg] of list.slice(Math.max(0, at))) {
            if (reg.hasNextPage) {
                reg.fetchNextPage()
                return
            }
        }
    }, [])

    const data = useMemo<GroupedGridData>(
        () => ({items, loadMore, hasMore: [...sections.values()].some((r) => r.hasNextPage)}),
        [items, loadMore, sections],
    )

    return (
        <ApiContext.Provider value={api}>
            <DataContext.Provider value={data}>{children}</DataContext.Provider>
        </ApiContext.Provider>
    )
}

/** The flattened visible order — subscribes to every section's data. */
export function useGroupedGrid(): GroupedGridData {
    return useContext(DataContext)
}

function useGroupedGridApi(): GroupedGridApi {
    const ctx = useContext(ApiContext)
    if (!ctx) throw new Error('useGroupedGridApi outside a GroupedGridProvider')
    return ctx
}

/** Read the flattened order on demand (shift-click), without re-rendering on every page. */
export function useOrderedIds(): () => string[] {
    return useGroupedGridApi().getOrderedIds
}

/** Register one section's slice; re-registers on data change and unregisters on collapse. */
export function useRegisterSection(key: string, reg: SectionRegistration): void {
    const {register, unregister} = useGroupedGridApi()
    const {order, items, fetchNextPage, hasNextPage} = reg
    const orderSig = order.join('|')

    useEffect(() => {
        register(key, {order: orderSig.split('|').map(Number), items, fetchNextPage, hasNextPage})
    }, [key, orderSig, items, fetchNextPage, hasNextPage, register])

    useEffect(() => () => unregister(key), [key, unregister])
}
