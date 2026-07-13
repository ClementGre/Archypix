import {type MouseEvent, useEffect, useMemo, useRef, useState} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {Ban, Check, ChevronRight, Equal, Hash, Images, Loader2, MoreHorizontal, Pencil, Plus, Share2} from 'lucide-react'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import {useAllTags} from '@/hooks/useTags'
import {RenameTagDialog} from '@/components/tags/RenameTagDialog'
import {CreateShareDialog} from '@/components/shares/CreateShareDialog'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import {cn} from '@/lib/utils'

interface TreeNode {
    label: string
    path: string // wire form
    children: TreeNode[]
}

/** Membership of a tag in the current compound filter. */
interface TagState {
    included: boolean
    exact: boolean
    excluded: boolean
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

/** Per-tag include / include-exactly / exclude controls in a `…` menu (each a toggle). */
function TagMenu({state, actions, path}: {
    state: TagState
    actions: TagActions
    path: string
}) {
    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <button
                    onClick={(e) => e.stopPropagation()}
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground/60 opacity-0 hover:bg-muted hover:text-foreground group-hover:opacity-100 data-[state=open]:opacity-100"
                    aria-label="Tag filter options"
                >
                    <MoreHorizontal className="h-3.5 w-3.5"/>
                </button>
            </DropdownMenuTrigger>
            {/* The content is portaled but stays in the React tree under the row's onClick, so item
                clicks would otherwise bubble to `pick` and reset the filter — stop them here. */}
            <DropdownMenuContent align="start" className="w-44" onClick={(e) => e.stopPropagation()}>
                <DropdownMenuItem onClick={() => actions.toggleInclude(path)}>
                    <Plus className="mr-2 h-3.5 w-3.5"/>
                    {state.included ? 'Remove include' : 'Include'}
                    {state.included && <Check className="ml-auto h-3.5 w-3.5"/>}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.toggleExact(path)}>
                    <Equal className="mr-2 h-3.5 w-3.5"/>
                    {state.exact ? 'Remove exact' : 'Include exactly'}
                    {state.exact && <Check className="ml-auto h-3.5 w-3.5"/>}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.toggleExclude(path)}>
                    <Ban className="mr-2 h-3.5 w-3.5"/>
                    {state.excluded ? 'Remove exclude' : 'Exclude'}
                    {state.excluded && <Check className="ml-auto h-3.5 w-3.5"/>}
                </DropdownMenuItem>
                <DropdownMenuSeparator/>
                <DropdownMenuItem onClick={() => actions.share(path)}>
                    <Share2 className="mr-2 h-3.5 w-3.5"/>
                    Share this tag…
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.rename(path)}>
                    <Pencil className="mr-2 h-3.5 w-3.5"/>
                    Rename tag…
                </DropdownMenuItem>
            </DropdownMenuContent>
        </DropdownMenu>
    )
}

interface TagActions {
    pick: (path: string) => void
    quickToggleInclude: (path: string) => void
    toggleInclude: (path: string) => void
    toggleExact: (path: string) => void
    toggleExclude: (path: string) => void
    remove: (path: string) => void
    rename: (path: string) => void
    share: (path: string) => void
}

function TreeRow({
                     node,
                     depth,
                     activeTag,
                     activeRef,
                     expanded,
                     toggle,
                     stateOf,
                     actions,
                 }: {
    node: TreeNode
    depth: number
    activeTag: string | null
    activeRef: (el: HTMLDivElement | null) => void
    expanded: Set<string>
    toggle: (path: string) => void
    stateOf: (path: string) => TagState
    actions: TagActions
}) {
    const hasChildren = node.children.length > 0
    const isOpen = expanded.has(node.path)
    const isActive = activeTag === node.path
    const st = stateOf(node.path)

    return (
        <div>
            <div
                ref={isActive ? activeRef : undefined}
                onClick={(e: MouseEvent) => {
                    // ⌘/Ctrl-click quick-toggles this tag in the include set (build "X and Y" fast).
                    if (e.metaKey || e.ctrlKey) actions.quickToggleInclude(node.path)
                    else actions.pick(node.path)
                }}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        actions.pick(node.path)
                    }
                }}
                className={cn(
                    'group flex cursor-pointer items-center gap-1 rounded-md py-1 pr-1 text-sm',
                    st.excluded
                        ? 'text-destructive/80 line-through'
                        : st.included || st.exact
                            ? 'bg-primary/10 text-primary'
                            : 'text-foreground hover:bg-muted',
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
                    {st.exact ? (
                        <Equal className="h-3.5 w-3.5 shrink-0 opacity-70" aria-label="exact"/>
                    ) : st.excluded ? (
                        <Ban className="h-3.5 w-3.5 shrink-0 opacity-70" aria-label="excluded"/>
                    ) : (
                        <Hash className="h-3.5 w-3.5 shrink-0 opacity-60"/>
                    )}
                    <span className="truncate">{node.label}</span>
                </div>
                <TagMenu state={st} actions={actions} path={node.path}/>
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
                            stateOf={stateOf}
                            actions={actions}
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
    const [renameTarget, setRenameTarget] = useState<string | null>(null)
    const [shareTag, setShareTag] = useState<string | null>(null)

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

    const without = (arr: string[], p: string) => arr.filter((x) => x !== p)

    const stateOf = (path: string): TagState => ({
        included: params.tag === path || params.include.includes(path),
        exact: params.exact.includes(path),
        excluded: params.exclude.includes(path),
    })

    const actions: TagActions = {
        // Plain click filters by this tag alone (replaces any compound filter), exiting hierarchies.
        pick: (path) => {
            refreshTags()
            update({tag: path, include: [], exclude: [], exact: [], hierarchy: null, hpath: ''})
        },
        quickToggleInclude: (path) => {
            const {included} = stateOf(path)
            if (included) actions.remove(path)
            else actions.toggleInclude(path)
        },
        toggleInclude: (path) => {
            refreshTags()
            const {included, exact} = stateOf(path)
            if (included && !exact) return actions.remove(path)
            // Adopt as an extra include; the primary `tag` stays as-is.
            if (params.tag === path) return
            update({
                include: [...new Set([...params.include, path])],
                exact: without(params.exact, path),
                exclude: without(params.exclude, path),
            })
        },
        toggleExact: (path) => {
            refreshTags()
            if (params.exact.includes(path)) return actions.remove(path)
            update({
                tag: params.tag === path ? null : params.tag,
                exact: [...new Set([...params.exact, path])],
                include: without(params.include, path),
                exclude: without(params.exclude, path),
            })
        },
        toggleExclude: (path) => {
            refreshTags()
            if (params.exclude.includes(path)) return actions.remove(path)
            update({
                tag: params.tag === path ? null : params.tag,
                exclude: [...new Set([...params.exclude, path])],
                include: without(params.include, path),
                exact: without(params.exact, path),
            })
        },
        remove: (path) => {
            refreshTags()
            update({
                tag: params.tag === path ? null : params.tag,
                include: without(params.include, path),
                exclude: without(params.exclude, path),
                exact: without(params.exact, path),
            })
        },
        rename: (path) => setRenameTarget(path),
        share: (path) => setShareTag(path),
    }

    const noFilter = !params.tag && !params.include.length && !params.exact.length && !params.exclude.length

    return (
        <div className="flex h-full flex-col">
            <button
                onClick={() => update({tag: null, include: [], exclude: [], exact: [], hierarchy: null, hpath: ''})}
                className={cn(
                    'mx-2 mt-2 flex items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium',
                    noFilter ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-muted',
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
                        stateOf={stateOf}
                        actions={actions}
                    />
                ))}
            </div>

            {renameTarget && (
                <RenameTagDialog
                    oldTag={renameTarget}
                    open={renameTarget !== null}
                    onOpenChange={(o) => !o && setRenameTarget(null)}
                />
            )}

            {/* Share a tag straight from its row menu — pre-fills the create-share tag. */}
            <CreateShareDialog
                open={shareTag !== null}
                onOpenChange={(o) => !o && setShareTag(null)}
                showTrigger={false}
                initialTag={shareTag ?? undefined}
            />
        </div>
    )
}
