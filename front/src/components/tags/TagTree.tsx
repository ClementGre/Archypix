import {type DragEvent, type MouseEvent, useEffect, useMemo, useRef, useState} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {
    ArrowDownUp,
    Ban,
    Check,
    ChevronDown,
    ChevronRight,
    ChevronUp,
    Equal,
    Hash,
    Images,
    Link2,
    Loader2,
    MoreHorizontal,
    Pencil,
    Plus,
    Share2,
} from 'lucide-react'
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {useBatchEditTags, useTagTree, useWriteTagMeta} from '@/hooks/useTags'
import {EditTagDialog} from '@/components/tags/EditTagDialog'
import {NewTagDialog} from '@/components/tags/NewTagDialog'
import {TagDropDialog, type TagDrop, undoAction} from '@/components/tags/TagDropDialog'
import {TagShareBadge, useTagShareIndex} from '@/components/tags/TagShareBadge'
import {CreateShareDialog} from '@/components/shares/CreateShareDialog'
import {PublicShareDialog} from '@/components/shares/PublicShareDialog'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import {collidingNames, countsFor, reorderWrites, type TagNode, type TrashView} from '@/lib/tagTree'
import {flushTagMetaFor} from '@/lib/tagMetaQueue'
import {areSiblings, useTagDragStore} from '@/stores/tagDrag'
import {cn, TagPath} from '@/lib/utils'
import {toast} from 'sonner'

/** Membership of a tag in the current compound filter. */
interface TagState {
    included: boolean
    exact: boolean
    excluded: boolean
}

function ancestorsOf(path: string | null): Set<string> {
    const set = new Set<string>()
    if (!path) return set
    let prefix = ''
    for (const label of path.split('.')) {
        prefix = prefix ? `${prefix}.${label}` : label
        set.add(prefix)
    }
    return set
}

interface TagActions {
    pick: (path: string) => void
    quickToggleInclude: (path: string) => void
    toggleInclude: (path: string) => void
    toggleExact: (path: string) => void
    toggleExclude: (path: string) => void
    remove: (path: string) => void
    edit: (path: string) => void
    newChild: (path: string | null) => void
    reorder: (path: string | null) => void
    share: (path: string) => void
    publicShare: (path: string) => void
    drop: (target: TagNode) => void
    move: (siblings: TagNode[], index: number, delta: number) => void
}

/** Per-tag filter toggles plus the structural actions, in a `…` menu. */
function TagMenu({state, actions, node}: { state: TagState; actions: TagActions; node: TagNode }) {
    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <button
                    onClick={(e) => e.stopPropagation()}
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground/60 opacity-0 hover:bg-muted hover:text-foreground group-hover:opacity-100 data-[state=open]:opacity-100"
                    aria-label="Tag options"
                >
                    <MoreHorizontal className="h-3.5 w-3.5"/>
                </button>
            </DropdownMenuTrigger>
            {/* The content is portaled but stays in the React tree under the row's onClick, so item
                clicks would otherwise bubble to `pick` and reset the filter — stop them here. */}
            <DropdownMenuContent align="start" className="w-52" onClick={(e) => e.stopPropagation()}>
                <DropdownMenuItem onClick={() => actions.toggleInclude(node.path)}>
                    <Plus className="mr-2 h-3.5 w-3.5"/>
                    {state.included ? 'Remove include' : 'Include'}
                    {state.included && <Check className="ml-auto h-3.5 w-3.5"/>}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.toggleExact(node.path)}>
                    <Equal className="mr-2 h-3.5 w-3.5"/>
                    {state.exact ? 'Remove exact' : 'Include exactly'}
                    {state.exact && <Check className="ml-auto h-3.5 w-3.5"/>}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.toggleExclude(node.path)}>
                    <Ban className="mr-2 h-3.5 w-3.5"/>
                    {state.excluded ? 'Remove exclude' : 'Exclude'}
                    {state.excluded && <Check className="ml-auto h-3.5 w-3.5"/>}
                </DropdownMenuItem>
                <DropdownMenuSeparator/>
                <DropdownMenuItem onClick={() => actions.newChild(node.path)}>
                    <Plus className="mr-2 h-3.5 w-3.5"/>
                    New subtag…
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.reorder(node.path)}>
                    <ArrowDownUp className="mr-2 h-3.5 w-3.5"/>
                    Reorder subtags
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.share(node.path)}>
                    <Share2 className="mr-2 h-3.5 w-3.5"/>
                    Share this tag…
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.publicShare(node.path)}>
                    <Link2 className="mr-2 h-3.5 w-3.5"/>
                    New public share link…
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => actions.edit(node.path)}>
                    <Pencil className="mr-2 h-3.5 w-3.5"/>
                    Edit tag…
                </DropdownMenuItem>
            </DropdownMenuContent>
        </DropdownMenu>
    )
}

function TreeRow({
                     node,
                     siblings,
                     index,
                     depth,
                     activeTag,
                     activeRef,
                     expanded,
                     toggle,
                     stateOf,
                     actions,
                     reorderUnder,
                     shareInfo,
                     collisions,
                     trash,
                 }: {
    node: TagNode
    siblings: TagNode[]
    index: number
    depth: number
    trash: TrashView
    activeTag: string | null
    activeRef: (el: HTMLDivElement | null) => void
    expanded: Set<string>
    toggle: (path: string) => void
    stateOf: (path: string) => TagState
    actions: TagActions
    /** The parent whose children are currently in reorder mode (`''` = root level). */
    reorderUnder: string | null
    shareInfo: (path: string) => ReturnType<ReturnType<typeof useTagShareIndex>>
    collisions: Set<string>
}) {
    const hasChildren = node.children.length > 0
    const isOpen = expanded.has(node.path)
    const isActive = activeTag === node.path
    const st = stateOf(node.path)
    const dragging = useTagDragStore((s) => s.selection !== null)
    const [over, setOver] = useState(false)
    const parentPath = node.path.slice(0, Math.max(0, node.path.lastIndexOf('.')))
    const reordering = reorderUnder !== null && reorderUnder === parentPath
    // `SharedToMe.*` is structural — photos can never be dropped onto it (§9).
    const droppable = dragging && !TagPath.isProtected(node.path)
    const empty = countsFor(node, trash).count === 0
    // A display name that hides the label is shown with the label beside it — and a sibling
    // collision forces the label visible even when it normally would not be (§5).
    const showLabel = node.name !== node.label || collisions.has(node.name)

    return (
        <div>
            <div
                ref={isActive ? activeRef : undefined}
                onClick={(e: MouseEvent) => {
                    if (reordering) return
                    // ⌘/Ctrl-click quick-toggles this tag in the include set (build "X and Y" fast).
                    if (e.metaKey || e.ctrlKey) actions.quickToggleInclude(node.path)
                    else actions.pick(node.path)
                }}
                onDragOver={(e: DragEvent) => {
                    if (!dragging) return
                    e.preventDefault()
                    e.dataTransfer.dropEffect = droppable ? 'copy' : 'none'
                    setOver(droppable)
                }}
                onDragLeave={() => setOver(false)}
                onDrop={(e: DragEvent) => {
                    setOver(false)
                    if (!droppable) return
                    e.preventDefault()
                    actions.drop(node)
                }}
                role="button"
                tabIndex={0}
                title={TagPath.toDisplay(node.path)}
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
                    over && 'ring-2 ring-primary ring-inset',
                    dragging && !droppable && 'cursor-not-allowed opacity-50',
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
                    {node.meta?.color ? (
                        <span
                            className="h-2.5 w-2.5 shrink-0 rounded-full"
                            style={{backgroundColor: node.meta.color}}
                            aria-hidden
                        />
                    ) : st.exact ? (
                        <Equal className="h-3.5 w-3.5 shrink-0 opacity-70" aria-label="exact"/>
                    ) : st.excluded ? (
                        <Ban className="h-3.5 w-3.5 shrink-0 opacity-70" aria-label="excluded"/>
                    ) : (
                        <Hash className="h-3.5 w-3.5 shrink-0 opacity-60"/>
                    )}
                    <span className={cn('truncate', empty && 'text-muted-foreground')}>{node.name}</span>
                    {showLabel && (
                        <span className="shrink-0 truncate text-[10px] text-muted-foreground/70">{node.label}</span>
                    )}
                </div>
                <TagShareBadge path={node.path} info={shareInfo(node.path)} onShare={actions.share}/>
                {reordering ? (
                    <span className="flex shrink-0 items-center" onClick={(e) => e.stopPropagation()}>
                        <button
                            className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-30"
                            disabled={index === 0}
                            onClick={() => actions.move(siblings, index, -1)}
                            aria-label="Move up"
                        >
                            <ChevronUp className="h-3.5 w-3.5"/>
                        </button>
                        <button
                            className="rounded p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground disabled:opacity-30"
                            disabled={index === siblings.length - 1}
                            onClick={() => actions.move(siblings, index, 1)}
                            aria-label="Move down"
                        >
                            <ChevronDown className="h-3.5 w-3.5"/>
                        </button>
                    </span>
                ) : (
                    <TagMenu state={st} actions={actions} node={node}/>
                )}
            </div>
            {hasChildren && isOpen && (
                <div>
                    {node.children.map((child, i) => (
                        <TreeRow
                            key={child.path}
                            node={child}
                            siblings={node.children}
                            index={i}
                            depth={depth + 1}
                            activeTag={activeTag}
                            activeRef={activeRef}
                            expanded={expanded}
                            toggle={toggle}
                            stateOf={stateOf}
                            actions={actions}
                            reorderUnder={reorderUnder}
                            shareInfo={shareInfo}
                            collisions={collidingNames(node.children)}
                            trash={trash}
                        />
                    ))}
                </div>
            )}
        </div>
    )
}

export function TagTree() {
    const {params, update} = useGalleryParams()
    // A tag whose pictures are all trashed disappears from the default view and comes back under
    // trash `include`/`only`, with its trashed counts and range (feature 34 §4, §13.9).
    const trash = params.trash as TrashView
    const {tree, metaByPath, isPending, isError, error} = useTagTree(trash)
    const queryClient = useQueryClient()
    const write = useWriteTagMeta()
    const shareIndex = useTagShareIndex()
    const edit = useBatchEditTags()
    const drag = useTagDragStore()

    const [editTarget, setEditTarget] = useState<string | null>(null)
    const [newTagParent, setNewTagParent] = useState<string | null | undefined>(undefined)
    const [reorderUnder, setReorderUnder] = useState<string | null>(null)
    const [shareTag, setShareTag] = useState<string | null>(null)
    const [publicShareTag, setPublicShareTag] = useState<string | null>(null)
    const [drop, setDrop] = useState<TagDrop | null>(null)

    const nameOf = (path: string) => metaByPath.get(path)?.display_name?.trim() || TagPath.leaf(path)

    // The tag list can drift as the pipeline assigns/removes tags in the background; refresh it
    // on interaction so navigating the tree keeps it current.
    const refreshTags = () => void queryClient.invalidateQueries({queryKey: queryKeys.tags()})

    const [expanded, setExpanded] = useState<Set<string>>(() => ancestorsOf(params.tag))
    const toggle = (path: string) => {
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

    // A reorder is the one queued write whose loss would be visible, so leaving the tree flushes it
    // (§4.1, §13.8).
    useEffect(() => {
        return () => {
            if (reorderUnder !== null) flushTagMetaFor(reorderUnder)
        }
    }, [reorderUnder])

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
        edit: (path) => setEditTarget(path),
        newChild: (path) => setNewTagParent(path),
        // Dragging under a parent sorted by anything but `manual` is meaningless, so entering
        // reorder mode switches it and says so (§7).
        reorder: (path) => {
            const key = path ?? ''
            const order = metaByPath.get(key)?.children_order ?? 'manual'
            if (order !== 'manual') {
                write({tag_path: key, children_order: 'manual'})
                toast.info('Subtags now sort manually')
            }
            setReorderUnder(reorderUnder === key ? null : key)
        },
        share: (path) => setShareTag(path),
        publicShare: (path) => setPublicShareTag(path),
        move: (siblings, index, delta) => {
            const to = index + delta
            if (to < 0 || to >= siblings.length) return
            for (const w of reorderWrites(siblings, index, to)) write(w)
        },
        drop: (target) => {
            if (!drag.selection) return
            const payload: TagDrop = {
                selection: drag.selection,
                count: drag.count,
                sourceTag: drag.sourceTag,
                targetTag: target.path,
                targetName: target.name,
                sourceName: drag.sourceTag ? nameOf(drag.sourceTag) : null,
            }
            drag.end()
            // A single photo onto a non-sibling tag is assigned silently; everything else confirms.
            const silent =
                payload.count === 1 &&
                !(payload.sourceTag && areSiblings(payload.sourceTag, payload.targetTag))
            if (!silent) return setDrop(payload)
            edit.mutate(
                {selection: payload.selection, add_tags: [payload.targetTag]},
                {
                    onSuccess: () =>
                        toast.success(`Added to ${payload.targetName}`, undoAction(payload, null, edit)),
                    onError: (e) => toast.error(apiErrorMessage(e)),
                },
            )
        },
    }

    const rootCollisions = useMemo(() => collidingNames(tree), [tree])
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

            <div className="mx-2 mt-1 flex items-center justify-between text-[11px] text-muted-foreground">
                <button className="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-muted hover:text-foreground"
                        onClick={() => actions.newChild(null)}>
                    <Plus className="h-3 w-3"/>
                    New tag
                </button>
                <button className="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-muted hover:text-foreground"
                        onClick={() => actions.reorder(null)}>
                    <ArrowDownUp className="h-3 w-3"/>
                    {reorderUnder === '' ? 'Done' : 'Reorder'}
                </button>
            </div>

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
                {tree.map((node, i) => (
                    <TreeRow
                        key={node.path}
                        node={node}
                        siblings={tree}
                        index={i}
                        depth={0}
                        activeTag={params.tag}
                        activeRef={(el) => (activeRowRef.current = el)}
                        expanded={expanded}
                        toggle={toggle}
                        stateOf={stateOf}
                        actions={actions}
                        reorderUnder={reorderUnder}
                        shareInfo={shareIndex}
                        collisions={rootCollisions}
                        trash={trash}
                    />
                ))}
            </div>

            {editTarget && (
                <EditTagDialog
                    tagPath={editTarget}
                    open={editTarget !== null}
                    onOpenChange={(o) => !o && setEditTarget(null)}
                />
            )}

            {newTagParent !== undefined && (
                <NewTagDialog
                    parentPath={newTagParent}
                    open
                    onOpenChange={(o) => !o && setNewTagParent(undefined)}
                />
            )}

            <TagDropDialog drop={drop} onClose={() => setDrop(null)}/>

            {/* Share a tag straight from its row menu — pre-fills the create-share tag. */}
            <CreateShareDialog
                open={shareTag !== null}
                onOpenChange={(o) => !o && setShareTag(null)}
                showTrigger={false}
                initialTag={shareTag ?? undefined}
            />

            {/* Create a public share link straight from the tag row menu. */}
            <PublicShareDialog
                open={publicShareTag !== null}
                onOpenChange={(o) => !o && setPublicShareTag(null)}
                showTrigger={false}
                initialTag={publicShareTag ?? undefined}
            />
        </div>
    )
}
