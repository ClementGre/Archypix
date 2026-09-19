// Tag display and ordering (feature 34 §5, §7).
//
// The rule: **the ltree path is never hidden on a surface that writes it**. A display name is
// additive — `display()` falls back to the decoded leaf label, which is what every surface showed
// before this feature.
//
// Ordering is resolved entirely here: `children_order`, `sort_index`, `display_name` and the
// derived dates all ship in the one app-start payload, so the client can sort any level of the tree
// without a query. The server never sorts tags.

import {TagPath} from '@/lib/utils'
import type {TagCounts, TagListItem, TagMeta, TagOrder} from '@/lib/types'

/** Gap between consecutive `sort_index` slots — sparse, so a drag usually writes one row (§7). */
export const SORT_STEP = 100

export interface TagNode {
    /** Wire (ltree) path; `''` is the root view (§3.3). */
    path: string
    /** Decoded ltree leaf label — shown muted beside an overridden display name. */
    label: string
    /** `meta.display_name ?? label`. */
    name: string
    meta: TagMeta | null
    live: TagCounts
    trashed?: TagCounts
    children: TagNode[]
}

/** `display(path) = meta.display_name ?? decodeLabel(leaf(path))` — the one resolution helper (§5). */
export function display(path: string, meta?: TagMeta | null): string {
    return meta?.display_name?.trim() || TagPath.leaf(path)
}

/**
 * The whole path, each segment resolved through `display()` (§5). A flat chip list has no tree to
 * place a tag in, so the leaf alone ("Vietnam") does not say *which* Vietnam — those surfaces show
 * the full path, with display names substituted rather than hidden.
 */
export function displayPath(path: string, metaByPath: Map<string, TagMeta>): string {
    if (!path) return ''
    const out: string[] = []
    let prefix = ''
    for (const segment of path.split('.')) {
        prefix = prefix ? `${prefix}.${segment}` : segment
        out.push(display(prefix, metaByPath.get(prefix)))
    }
    return `/${out.join('/')}`
}

/** Which trash filter a view is under; picks the count/date half to sort and show by (§4). */
export type TrashView = 'exclude' | 'include' | 'only'

/** The counts a view reads: *All* takes the union of both halves. */
export function countsFor(node: TagNode, trash: TrashView): TagCounts {
    const {live, trashed} = node
    if (trash === 'only') return trashed ?? {count: 0, exact_count: 0, date_from: null, date_to: null}
    if (trash === 'exclude' || !trashed) return live
    return {
        count: live.count + trashed.count,
        exact_count: live.exact_count + trashed.exact_count,
        date_from: minDate(live.date_from, trashed.date_from),
        date_to: maxDate(live.date_to, trashed.date_to),
    }
}

function minDate(a: string | null, b: string | null): string | null {
    if (!a || !b) return a ?? b
    return a < b ? a : b
}

function maxDate(a: string | null, b: string | null): string | null {
    if (!a || !b) return a ?? b
    return a > b ? a : b
}

/**
 * A tag is visible when it has pictures in the active trash view, or when it was deliberately
 * created empty. A tag whose pictures are all trashed disappears from the default view and comes
 * back under trash `include`/`only` (§4, §13.9) — otherwise trashing an event would leave a
 * permanent empty node behind.
 */
function isVisible(node: TagNode, trash: TrashView): boolean {
    if (node.meta?.show_when_empty) return true
    if (node.children.length > 0) return true
    return countsFor(node, trash).count > 0
}

/**
 * Build the tree from the flat payload and resolve every level's order. Paths are
 * ancestor-expanded server-side, but an intermediate can still be missing (an empty tag's ancestor),
 * so nodes are synthesized as needed.
 */
export function buildTagTree(items: TagListItem[], trash: TrashView = 'exclude'): TagNode[] {
    const byPath = new Map<string, TagNode>()
    const roots: TagNode[] = []
    const zero: TagCounts = {count: 0, exact_count: 0, date_from: null, date_to: null}

    const ensure = (path: string): TagNode => {
        const found = byPath.get(path)
        if (found) return found
        const node: TagNode = {
            path,
            label: TagPath.leaf(path),
            name: TagPath.leaf(path),
            meta: null,
            live: {...zero},
            children: [],
        }
        byPath.set(path, node)
        const cut = path.lastIndexOf('.')
        if (cut < 0) roots.push(node)
        else ensure(path.slice(0, cut)).children.push(node)
        return node
    }

    for (const item of items) {
        if (!item.path) continue // the root view is not a tree node
        const node = ensure(item.path)
        node.live = {
            count: item.count,
            exact_count: item.exact_count,
            date_from: item.date_from,
            date_to: item.date_to,
        }
        node.trashed = item.trashed
        node.meta = item.meta
        node.name = display(item.path, item.meta)
    }

    const prune = (nodes: TagNode[]): TagNode[] =>
        nodes
            .map((n) => ({...n, children: prune(n.children)}))
            .filter((n) => isVisible(n, trash))

    const sortRec = (nodes: TagNode[], parent: TagMeta | null | undefined): TagNode[] => {
        const sorted = sortSiblings(
            nodes,
            parent?.children_order ?? 'manual',
            trash,
            parent?.children_order_desc ?? false,
        )
        return sorted.map((n) => ({...n, children: sortRec(n.children, n.meta)}))
    }

    return sortRec(prune(roots), items.find((i) => i.path === '')?.meta)
}

/**
 * Sort one sibling set under a parent's `children_order` and its direction (§7).
 *
 * Manual order is `sort_index` ascending with unset last, tie-broken by the display name, so
 * **manual and alphabetical coincide until the first drag** — reordering needs no mode switch and
 * an unconfigured tree is alphabetical, as expected.
 */
export function sortSiblings(
    nodes: TagNode[],
    order: TagOrder,
    trash: TrashView,
    desc = false,
): TagNode[] {
    const sign = desc ? -1 : 1
    const byName = (a: TagNode, b: TagNode) => sign * a.name.localeCompare(b.name)
    /** A tag with nothing to sort by stays last in **both** directions. */
    const byValue =
        (value: (n: TagNode) => string | number | null | undefined) =>
        (a: TagNode, b: TagNode) => {
            const av = value(a)
            const bv = value(b)
            if (av == null || bv == null) return (av == null ? 1 : 0) - (bv == null ? 1 : 0) || byName(a, b)
            return av === bv ? byName(a, b) : sign * (av < bv ? -1 : 1)
        }
    const out = [...nodes]
    switch (order) {
        case 'path':
            return out.sort((a, b) => sign * a.path.localeCompare(b.path))
        case 'display_name':
            return out.sort(byName)
        case 'date_from':
        case 'date_to':
            return out.sort(byValue((n) => effectiveDate(n, order, trash)))
        case 'manual':
        default:
            return out.sort(byValue((n) => n.meta?.sort_index))
    }
}

/** An explicit override replaces the derived value on that side only, in every trash mode (§4). */
export function effectiveDate(
    node: TagNode,
    side: 'date_from' | 'date_to',
    trash: TrashView,
): string | null {
    return node.meta?.[side] ?? countsFor(node, trash)[side]
}

/**
 * The `sort_index` to write for a node dropped at `toIndex` in `siblings` (§7).
 *
 * A sibling with no stored index has an *effective* index of `position × SORT_STEP`, which is what
 * makes the first drag in an unconfigured list a single-row write instead of materialising a row
 * per sibling. When two neighbours leave no integer between them the whole list is renumbered in
 * sparse steps — rare, and still one request.
 */
export function reorderWrites(
    siblings: TagNode[],
    fromIndex: number,
    toIndex: number,
): Array<{ tag_path: string; sort_index: number }> {
    if (fromIndex === toIndex) return []
    // A partially-numbered list cannot be nudged one row at a time: unset indices sort *last*, so
    // the single row that gains one jumps to the front instead of moving. Number the list first.
    if (siblings.some((n) => n.meta?.sort_index == null)) {
        return renumber(siblings, fromIndex, toIndex)
    }
    const effective = siblings.map((n, i) => n.meta?.sort_index ?? i * SORT_STEP)
    const moved = siblings[fromIndex]
    const rest = effective.filter((_, i) => i !== fromIndex)
    const before = toIndex > 0 ? rest[toIndex - 1] : undefined
    const after = rest[toIndex]

    const lo = before ?? (after != null ? after - 2 * SORT_STEP : 0)
    const hi = after ?? lo + 2 * SORT_STEP
    const slot = Math.floor((lo + hi) / 2)
    if (slot > lo && slot < hi) return [{tag_path: moved.path, sort_index: slot}]

    // No integer left between the neighbours — renumber this one sibling list.
    return renumber(siblings, fromIndex, toIndex)
}

/** Write a sparse index for every sibling, optionally moving one of them first. */
function renumber(
    siblings: TagNode[],
    fromIndex?: number,
    toIndex?: number,
): Array<{ tag_path: string; sort_index: number }> {
    const order = [...siblings]
    if (fromIndex != null && toIndex != null) order.splice(toIndex, 0, ...order.splice(fromIndex, 1))
    return order.map((n, i) => ({tag_path: n.path, sort_index: i * SORT_STEP}))
}

/**
 * Materialise the current resolved order as explicit indices (§7). Entering reorder mode calls this
 * so the list starts numbered — otherwise the first move is a no-op (see `reorderWrites`). Returns
 * nothing when every sibling already has one, unless `force` — leaving a descending order writes
 * ascending indices along the list *as displayed*, so it does not flip under the user.
 */
export function initialOrderWrites(
    siblings: TagNode[],
    force = false,
): Array<{ tag_path: string; sort_index: number }> {
    if (!force && !siblings.some((n) => n.meta?.sort_index == null)) return []
    return renumber(siblings)
}

/** Sibling display-name collisions are allowed and warned, but must not be indistinguishable in a
 *  picker — when one exists the ltree label is forced visible on every colliding entry (§5). */
export function collidingNames(siblings: TagNode[]): Set<string> {
    const seen = new Map<string, number>()
    for (const n of siblings) seen.set(n.name, (seen.get(n.name) ?? 0) + 1)
    return new Set([...seen].filter(([, n]) => n > 1).map(([name]) => name))
}

/** Depth-first walk, parents before children. */
export function walkTags(nodes: TagNode[], visit: (node: TagNode, parent: TagNode | null) => void): void {
    const rec = (list: TagNode[], parent: TagNode | null) => {
        for (const n of list) {
            visit(n, parent)
            rec(n.children, n)
        }
    }
    rec(nodes, null)
}
