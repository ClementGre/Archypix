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
} from '@/lib/types'

/** Generate a stable, hierarchy-unique node id (`n_` + 8 hex chars). */
export function genNodeId(): string {
    return 'n_' + Math.random().toString(16).slice(2, 10)
}

/** Empty config matching the server's normalized default (§4.1, v2). */
export function emptyConfig(): HierarchyConfig {
    return {
        version: 2,
        safeDeleteMode: 'singleBranch',
        naming: 'original',
        writeBack: true,
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

export function makeDropNode(): DropNode {
    return {id: genNodeId(), kind: 'drop', name: 'Inbox', onAdd: [{op: 'assign', path: 'Inbox'}]}
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

export const NAMING_OPTIONS: { value: NamingStrategy; label: string }[] = [
    {value: 'original', label: 'Original filename'},
    {value: 'date', label: 'Capture date'},
    {value: 'id', label: 'Picture id'},
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
    mirror: 'Expands the live tag subtree under a tag root into directories.',
    query: 'A tag predicate; may nest sub-folders. Writable when write-back is on.',
    static: 'A pure container folder — holds sub-folders, no pictures of its own.',
    drop: 'A write-only inbox: lists nothing, tags every upload. Always writable.',
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
