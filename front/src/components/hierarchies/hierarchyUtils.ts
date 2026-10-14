import type {
    DropNode,
    HierarchyConfig,
    HierarchyNode,
    MirrorNode,
    NamingStrategy,
    NodeKind,
    QueryNode,
    SafeDeleteMode,
    StaticNode,
    TrashedDuplicate,
} from '@/lib/types'

/** Generate a stable, hierarchy-unique node id (`n_` + 8 hex chars). */
export function genNodeId(): string {
    return 'n_' + Math.random().toString(16).slice(2, 10)
}

/** Empty config matching the server's normalized default (§4.1, v3). */
export function emptyConfig(): HierarchyConfig {
    return {
        version: 3,
        safeDeleteMode: 'singleBranch',
        naming: 'original',
        writeBack: true,
        trashedDuplicate: 'restore',
        nodes: [],
    }
}

export function makeMirrorNode(): MirrorNode {
    return {id: genNodeId(), kind: 'mirror', name: 'Photos', tagRoot: 'Photos', keepDir: false}
}

export function makeQueryNode(): QueryNode {
    return {id: genNodeId(), kind: 'query', name: 'New folder', match: 'all', include: []}
}

export function makeStaticNode(): StaticNode {
    return {id: genNodeId(), kind: 'static', name: 'New folder', children: []}
}

/** Created with an explicit `ignore` so the trashed-duplicate behaviour is visible (feature 39 §6). */
export function makeDropNode(): DropNode {
    return {
        id: genNodeId(),
        kind: 'drop',
        name: 'Inbox',
        trashedDuplicate: 'ignore',
        onAdd: [{op: 'assign', path: 'Inbox'}],
    }
}

export function makeNode(kind: NodeKind): HierarchyNode {
    if (kind === 'mirror') return makeMirrorNode()
    if (kind === 'query') return makeQueryNode()
    if (kind === 'drop') return makeDropNode()
    return makeStaticNode()
}

/**
 * Effective write-back for a node given the master switch and the nearest explicit ancestor
 * value (feature 18 §5.1). `master` off is a hard ceiling; otherwise the node's own
 * `writeBackEnabled` (if set) wins, else it inherits. Mirrors the backend `effective_enabled`.
 */
export function effectiveWriteBack(
    master: boolean,
    inherited: boolean,
    writeBackEnabled: boolean | null | undefined,
): boolean {
    if (!master) return false
    return writeBackEnabled ?? inherited
}

/**
 * The Android camera-sync recipe (feature 39 §8.1): a listing `Camera` folder over a permanent
 * provenance tag, deleting from it only removes it from the folder.
 */
export function cameraSyncConfig(): HierarchyConfig {
    return {
        ...emptyConfig(),
        nodes: [{
            id: genNodeId(),
            kind: 'query',
            name: 'Camera',
            match: 'all',
            include: ['Camera'],
            safeDeleteMode: 'singleBranch',
            writeBack: {
                onAdd: [{op: 'assign', path: 'Camera'}],
                onRemove: [{op: 'remove', path: 'Camera'}],
            },
        }],
    }
}

/** What a node inherits from its ancestors (feature 18 §5.1, feature 39 §6). */
export interface Inherited {
    master: boolean
    writeBack: boolean
    trashedDuplicate: TrashedDuplicate
}

/** A node's authored children (query/static), if any. */
export function childrenOf(node: HierarchyNode): HierarchyNode[] | undefined {
    return canHaveChildren(node) ? node.children ?? [] : undefined
}

function withChildren(node: HierarchyNode, children: HierarchyNode[]): HierarchyNode {
    return canHaveChildren(node) ? {...node, children: children.length ? children : undefined} : node
}

export function findNode(nodes: HierarchyNode[], id: string): HierarchyNode | null {
    for (const n of nodes) {
        if (n.id === id) return n
        const found = findNode(childrenOf(n) ?? [], id)
        if (found) return found
    }
    return null
}

/** Replace the node `id` with `fn(node)`, or remove it when `fn` returns null. */
export function mapNode(
    nodes: HierarchyNode[],
    id: string,
    fn: (n: HierarchyNode) => HierarchyNode | null,
): HierarchyNode[] {
    return nodes.flatMap((n) => {
        if (n.id === id) {
            const next = fn(n)
            return next ? [next] : []
        }
        const kids = childrenOf(n)
        return kids ? [withChildren(n, mapNode(kids, id, fn))] : [n]
    })
}

/** Swap `id` with its previous (`-1`) or next (`1`) sibling. */
export function moveNode(nodes: HierarchyNode[], id: string, dir: -1 | 1): HierarchyNode[] {
    const i = nodes.findIndex((n) => n.id === id)
    if (i >= 0) {
        const j = i + dir
        if (j < 0 || j >= nodes.length) return nodes
        const next = [...nodes]
        ;[next[i], next[j]] = [next[j], next[i]]
        return next
    }
    return nodes.map((n) => {
        const kids = childrenOf(n)
        return kids ? withChildren(n, moveNode(kids, id, dir)) : n
    })
}

/** Append `child` under `parentId` (`null` = top level). */
export function addChild(nodes: HierarchyNode[], parentId: string | null, child: HierarchyNode): HierarchyNode[] {
    if (parentId === null) return [...nodes, child]
    return mapNode(nodes, parentId, (p) => withChildren(p, [...(childrenOf(p) ?? []), child]))
}

/** What node `id` inherits, resolved down from the hierarchy's root seeds; null if absent. */
export function inheritedFor(config: HierarchyConfig, id: string): Inherited | null {
    const master = config.writeBack
    const walk = (list: HierarchyNode[], up: Inherited): Inherited | null => {
        for (const n of list) {
            if (n.id === id) return up
            const kids = childrenOf(n)
            if (!kids) continue
            const here: Inherited = {
                master,
                writeBack: effectiveWriteBack(master, up.writeBack, n.writeBackEnabled),
                trashedDuplicate: n.trashedDuplicate ?? up.trashedDuplicate,
            }
            const found = walk(kids, here)
            if (found) return found
        }
        return null
    }
    return walk(config.nodes, {master, writeBack: true, trashedDuplicate: config.trashedDuplicate ?? 'restore'})
}

export const NAMING_OPTIONS: { value: NamingStrategy; label: string }[] = [
    {value: 'original', label: 'Original filename'},
    {value: 'date', label: 'Capture date'},
    {value: 'id', label: 'Picture id'},
]

export const TRASHED_DUPLICATE_OPTIONS: { value: TrashedDuplicate; label: string }[] = [
    {value: 'restore', label: 'Restore it and tag it'},
    {value: 'ignore', label: 'Leave it in the trash'},
]

export const SAFE_DELETE_OPTIONS: { value: SafeDeleteMode; label: string }[] = [
    {value: 'singleBranch', label: 'Single branch (remove tag only)'},
    {value: 'fullDelete', label: 'Full delete (move to trash)'},
]

export const KIND_LABEL: Record<NodeKind, string> = {
    mirror: 'Mirror',
    query: 'Query',
    static: 'Static',
    drop: 'Drop',
}

export const KIND_COLOR: Record<NodeKind, string> = {
    mirror: 'bg-sky-500/15 text-sky-500',
    query: 'bg-violet-500/15 text-violet-500',
    static: 'bg-amber-500/15 text-amber-500',
    drop: 'bg-emerald-500/15 text-emerald-500',
}

export const KIND_HINT: Record<NodeKind, string> = {
    mirror: 'Turns the tags under a tag root into folders.',
    query: 'Shows the pictures matching some tags. Can hold sub-folders.',
    static: 'A plain folder that only holds sub-folders.',
    drop: 'An upload inbox. Tags every upload and lists only recent ones.',
}

/** The display name a node renders as (mirror defaults to its tagRoot leaf). */
export function nodeDisplayName(node: HierarchyNode): string {
    if (node.name) return node.name
    if (node.kind === 'mirror') {
        const parts = node.tagRoot.split('.')
        return parts[parts.length - 1] || node.tagRoot
    }
    return '(unnamed)'
}

/** Whether a node kind can hold authored children. */
export function canHaveChildren(node: HierarchyNode): node is QueryNode | StaticNode {
    return node.kind === 'query' || node.kind === 'static'
}
