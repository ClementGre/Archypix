import {useEffect, useMemo, useRef, useState} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {ChevronRight, Hash, Images, Loader2} from 'lucide-react'
import {useAllTags} from '@/hooks/useTags'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import {cn} from '@/lib/utils'

interface TreeNode {
    label: string
    path: string // wire form
    children: TreeNode[]
}

function decodeLabel(label: string): string {
    return label.replace(/_AT_/g, '@').replace(/_DOT_/g, '.')
}

function buildTree(paths: string[]): TreeNode[] {
    const roots: TreeNode[] = []
    for (const path of paths) {
        let level = roots
        let prefix = ''
        for (const label of path.split('.')) {
            prefix = prefix ? `${prefix}.${label}` : label
            let node = level.find((n) => n.path === prefix)
            if (!node) {
                node = {label: decodeLabel(label), path: prefix, children: []}
                level.push(node)
            }
            level = node.children
        }
    }
    const sortRec = (nodes: TreeNode[]) => {
        nodes.sort((a, b) => a.label.localeCompare(b.label))
        nodes.forEach((n) => sortRec(n.children))
    }
    sortRec(roots)
    return roots
}

function ancestorsOf(path: string | null): Set<string> {
    const set = new Set<string>()
    if (!path) return set
    const labels = path.split('.')
    let prefix = ''
    for (const label of labels) {
        prefix = prefix ? `${prefix}.${label}` : label
        set.add(prefix)
    }
    return set
}

function TreeRow({
                     node,
                     depth,
                     activeTag,
                     activeRef,
                     expanded,
                     toggle,
                     onPick,
                 }: {
    node: TreeNode
    depth: number
    activeTag: string | null
    activeRef: (el: HTMLDivElement | null) => void
    expanded: Set<string>
    toggle: (path: string) => void
    onPick: (path: string) => void
}) {
    const hasChildren = node.children.length > 0
    const isOpen = expanded.has(node.path)
    const isActive = activeTag === node.path

    return (
        <div>
            <div
                ref={isActive ? activeRef : undefined}
                onClick={() => onPick(node.path)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        onPick(node.path)
                    }
                }}
                className={cn(
                    'group flex cursor-pointer items-center gap-1 rounded-md py-1 pr-2 text-sm',
                    isActive ? 'bg-primary/10 text-primary' : 'text-foreground hover:bg-muted',
                )}
                style={{paddingLeft: depth * 12 + 4}}
            >
                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        if (hasChildren) toggle(node.path)
                    }}
                    className={cn('flex h-4 w-4 shrink-0 items-center justify-center', !hasChildren && 'invisible')}
                    aria-label={isOpen ? 'Collapse' : 'Expand'}
                >
                    <ChevronRight className={cn('h-3.5 w-3.5 transition-transform', isOpen && 'rotate-90')}/>
                </button>
                <div className="flex min-w-0 flex-1 items-center gap-1.5">
                    <Hash className="h-3.5 w-3.5 shrink-0 opacity-60"/>
                    <span className="truncate">{node.label}</span>
                </div>
            </div>
            {hasChildren && isOpen && (
                <div>
                    {node.children.map((child) => (
                        <TreeRow
                            key={child.path}
                            node={child}
                            depth={depth + 1}
                            activeTag={activeTag}
                            activeRef={activeRef}
                            expanded={expanded}
                            toggle={toggle}
                            onPick={onPick}
                        />
                    ))}
                </div>
            )}
        </div>
    )
}

export function TagTree() {
    const {data: tags, isPending, isError, error} = useAllTags()
    const {params, update} = useGalleryParams()
    const queryClient = useQueryClient()
    const tree = useMemo(() => buildTree(tags ?? []), [tags])

    // The tag list can drift as the pipeline assigns/removes tags in the background; refresh it
    // on interaction so navigating the tree keeps it current.
    const refreshTags = () => void queryClient.invalidateQueries({queryKey: queryKeys.tags()})

    const [expanded, setExpanded] = useState<Set<string>>(() => ancestorsOf(params.tag))
    const toggle = (path: string) => {
        refreshTags()
        setExpanded((prev) => {
            const next = new Set(prev)
            if (next.has(path)) next.delete(path)
            else next.add(path)
            return next
        })
    }

    // When the active tag changes (e.g. via a cross-link), expand its ancestors so
    // it becomes visible without collapsing what the user has already opened.
    useEffect(() => {
        if (!params.tag) return
        setExpanded((prev) => {
            const next = new Set(prev)
            for (const anc of ancestorsOf(params.tag)) next.add(anc)
            return next
        })
    }, [params.tag])

    // Scroll the active row into view once it (and its ancestors) are expanded.
    const activeRowRef = useRef<HTMLDivElement | null>(null)
    useEffect(() => {
        if (params.tag) activeRowRef.current?.scrollIntoView({block: 'nearest'})
    }, [params.tag, expanded])

    // Picking a tag exits any active hierarchy view — filtering by a tag inside a
    // hierarchy directory is confusing, so clear the hierarchy params.
    const pick = (path: string) => {
        refreshTags()
        update({tag: path, hierarchy: null, hpath: ''})
    }

    return (
        <div className="flex h-full flex-col">
            <button
                onClick={() => update({tag: null, hierarchy: null, hpath: ''})}
                className={cn(
                    'mx-2 mt-2 flex items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium',
                    !params.tag ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-muted',
                )}
            >
                <Images className="h-4 w-4"/>
                All photos
            </button>

            <div className="mt-1 flex-1 overflow-y-auto px-1 pb-2">
                {isPending && (
                    <div className="flex items-center justify-center py-6 text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin"/>
                    </div>
                )}
                {isError && <p className="px-3 py-4 text-xs text-muted-foreground">{apiErrorMessage(error)}</p>}
                {!isPending && !isError && tree.length === 0 && (
                    <p className="px-3 py-4 text-xs text-muted-foreground">No tags yet.</p>
                )}
                {tree.map((node) => (
                    <TreeRow
                        key={node.path}
                        node={node}
                        depth={0}
                        activeTag={params.tag}
                        activeRef={(el) => (activeRowRef.current = el)}
                        expanded={expanded}
                        toggle={toggle}
                        onPick={pick}
                    />
                ))}
            </div>
        </div>
    )
}
