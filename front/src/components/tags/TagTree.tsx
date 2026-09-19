import {type DragEvent, type MouseEvent, useEffect, useMemo, useRef, useState} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {
    ArrowDown,
    ArrowDownUp,
    ArrowUp,
    Ban,
    Check,
    ChevronDown,
    ChevronRight,
    ChevronUp,
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
    DropdownMenuRadioGroup,
    DropdownMenuRadioItem,
    DropdownMenuSeparator,
    DropdownMenuSub,
    DropdownMenuSubContent,
    DropdownMenuSubTrigger,
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
import {collidingNames, countsFor, initialOrderWrites, reorderWrites, type TagNode, type TrashView} from '@/lib/tagTree'
import type {TagOrder} from '@/lib/types'
import {flushTagMetaFor} from '@/lib/tagMetaQueue'
import {areSiblings, useTagDragStore} from '@/stores/tagDrag'
import {cn, TagPath} from '@/lib/utils'
import {toast} from 'sonner'

/** Membership of a tag in the current compound filter. */
interface TagState {
    included: boolean
    excluded: boolean
}

/** How a parent's children sort (34 §7). Manual and alphabetical coincide until the first drag. */
const ORDERS: { value: TagOrder; label: string }[] = [
    {value: 'manual', label: 'Custom order'},
    {value: 'display_name', label: 'Display name'},
    {value: 'path', label: 'Tag name'},
    {value: 'date_from', label: 'Start date'},
    {value: 'date_to', label: 'End date'},
]

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
    toggleExclude: (path: string) => void
    remove: (path: string) => void
    edit: (path: string) => void
    newChild: (path: string | null) => void
    reorder: (path: string | null) => void
    setOrder: (path: string | null, order: TagOrder) => void
    setOrderDesc: (path: string | null, desc: boolean) => void
    share: (path: string) => void
    publicShare: (path: string) => void
    drop: (target: TagNode) => void
    move: (siblings: TagNode[], index: number, delta: number) => void
}

interface OrderProps {
    path: string | null
    current: TagOrder
    desc: boolean
    actions: TagActions
}

/** The order choices themselves; `manual` additionally offers the explicit reorder mode (34 §7). */
function OrderChoices({path, current, desc, actions}: OrderProps) {
    return (
        <>
            <DropdownMenuRadioGroup value={current} onValueChange={(v) => actions.setOrder(path, v as TagOrder)}>
                {ORDERS.map((o) => (
                    <DropdownMenuRadioItem key={o.value} value={o.value}>{o.label}</DropdownMenuRadioItem>
                ))}
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator/>
            <DropdownMenuRadioGroup value={desc ? 'desc' : 'asc'}
                                    onValueChange={(v) => actions.setOrderDesc(path, v === 'desc')}>
                <DropdownMenuRadioItem value="asc">Ascending</DropdownMenuRadioItem>
                <DropdownMenuRadioItem value="desc">Descending</DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator/>
            <DropdownMenuItem onClick={() => actions.reorder(path)}>
                <ArrowDownUp className="mr-2 h-3.5 w-3.5"/>
                Reorder manually…
            </DropdownMenuItem>
        </>
    )
}

/** The same choices as a submenu, for a node's `…` menu. */
function OrderSubmenu(props: OrderProps) {
    return (
        <DropdownMenuSub>
            <DropdownMenuSubTrigger>
                <ArrowDownUp className="mr-2 h-3.5 w-3.5"/>
                Sort subtags by
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent>
                <OrderChoices {...props} />
            </DropdownMenuSubContent>
        </DropdownMenuSub>
    )
}

/** Per-tag filter toggles plus the structural actions, in a `…` menu. Replaces by the pictures count when not hovered/small screen */
function TagMenu({count, state, actions, node}: { count: number, state: TagState; actions: TagActions; node: TagNode }) {
    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <button
                    onClick={(e) => e.stopPropagation()}
                    // `min-w-5` so a wide count is not clipped by the icon's footprint; the group is
                    // what lets the two swap while the menu is open.
                    className="group/menu flex h-5 min-w-5 shrink-0 items-center justify-center rounded px-0.5 text-muted-foreground/60 hover:bg-muted hover:text-foreground"
                    aria-label="Tag options"
                >
                    <span className="hidden text-[11px] tabular-nums text-muted-foreground md:inline md:group-hover:hidden group-data-[state=open]/menu:md:hidden">
                        {count}
                    </span>
                    {/* Always visible on touch, where there is no hover to reveal it. */}
                    <MoreHorizontal className="h-3.5 w-3.5 md:hidden md:group-hover:block group-data-[state=open]/menu:md:block"/>
                </button>
            </DropdownMenuTrigger>
            {/* The content is portaled but stays in the React tree under the row's onClick, so item
                clicks would otherwise bubble to `pick` and reset the filter — stop them here. */}
            <DropdownMenuContent align="start" className="w-56" onClick={(e) => e.stopPropagation()}>
                {/* The row shows only the display name, so this is where the ltree path stays
                    visible — the rule is that it is never hidden on a surface that writes it (§5). */}
                <div className="border-b border-border px-2 pb-1.5">
                    <p className="truncate text-xs font-medium">{node.name}</p>
                    <p className="truncate font-mono text-[10px] text-muted-foreground"
                       title={TagPath.toDisplay(node.path)}>
                        {TagPath.toDisplay(node.path)}
                    </p>
                </div>
                <DropdownMenuItem onClick={() => actions.toggleInclude(node.path)}>
                    <Plus className="mr-2 h-3.5 w-3.5"/>
                    {state.included ? 'Remove include' : 'Include'}
                    {state.included && <Check className="ml-auto h-3.5 w-3.5"/>}
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
                <OrderSubmenu path={node.path} current={node.meta?.children_order ?? 'manual'}
                              desc={node.meta?.children_order_desc ?? false} actions={actions}/>
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
    const count = countsFor(node, trash).count
    const empty = count === 0
    const color = node.meta?.color ?? null
    // A coloured row keeps its own colour when selected — deeper tint, solid accent, bolder text —
    // rather than swapping to the primary accent, which reads as a different tag.
    const tint = st.excluded ? null : color

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
                    'group flex cursor-pointer items-center gap-1 rounded-md border-l-2 py-1 pr-1 text-sm',
                    st.excluded
                        ? 'text-destructive/80 line-through'
                        : tint
                            ? cn('text-foreground', st.included && 'font-medium')
                            : st.included
                                ? 'bg-primary/10 text-primary'
                                : 'text-foreground hover:bg-muted',
                    over && 'ring-2 ring-primary ring-inset',
                    dragging && !droppable && 'cursor-not-allowed opacity-50',
                )}
                // The colour reads as a left accent on the row plus a tint on the hash, rather than
                // a bare dot standing in for the icon.
                style={{
                    paddingLeft: depth * 12 + 4,
                    borderLeftColor: tint ? (st.included ? tint : `${tint}80`) : (color ?? 'transparent'),
                    ...(tint ? {backgroundColor: `${tint}${st.included ? '3d' : '14'}`} : {}),
                    // Included: the label takes a variant of the tag's own colour, mixed toward the
                    // theme foreground so it darkens in light mode and lightens in dark. The colour
                    // changes on selection as `text-primary` does for an uncoloured tag, and the
                    // contrast against the tint is the theme's, not the palette's.
                    ...(tint && st.included
                        ? {color: `color-mix(in oklab, ${tint} 62%, var(--color-foreground))`}
                        : {}),
                }}
            >
                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        if (hasChildren && !reordering) toggle(node.path)
                    }}
                    className={cn('flex h-4 w-4 shrink-0 items-center justify-center', !hasChildren && 'invisible')}
                    aria-label={isOpen ? 'Collapse' : 'Expand'}
                >
                    <ChevronRight className={cn('h-3.5 w-3.5 transition-transform', isOpen && 'rotate-90', reordering && 'opacity-20')}/>
                </button>
                <div className="flex min-w-0 flex-1 items-center gap-1.5">
                    {st.excluded ? (
                        <Ban className="h-3.5 w-3.5 shrink-0 opacity-70" aria-label="excluded"/>
                    ) : (
                        <Hash
                            className={cn('h-3.5 w-3.5 shrink-0', color ? 'opacity-100' : 'opacity-60')}
                            style={color ? {color} : undefined}
                        />
                    )}
                    <span className={cn('truncate', empty && 'text-muted-foreground')}>{node.name}</span>
                    {/* Two identically-named siblings must not be indistinguishable (§5) — that is
                        the one case where the ltree label still shows on the row. */}
                    {collisions.has(node.name) && (
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
                ) : <TagMenu count={count} state={st} actions={actions} node={node}/>}

            </div>
            {hasChildren && isOpen && !reordering && (
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

    /** The sibling set under a parent path; `''` is the root level. */
    const childrenOf = (parent: string): TagNode[] => {
        if (!parent) return tree
        let level = tree
        for (const anc of ancestorsOf(parent)) {
            const found = level.find((n) => n.path === anc)
            if (!found) return []
            if (anc === parent) return found.children
            level = found.children
        }
        return []
    }

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
        excluded: params.exclude.includes(path),
    })

    const actions: TagActions = {
        // Plain click makes this tag the view root (replaces any compound filter), exiting hierarchies.
        pick: (path) => {
            refreshTags()
            update({tag: path, include: [], exclude: [], hierarchy: null, hpath: ''})
        },
        quickToggleInclude: (path) => {
            const {included} = stateOf(path)
            if (included) actions.remove(path)
            else actions.toggleInclude(path)
        },
        toggleInclude: (path) => {
            refreshTags()
            if (stateOf(path).included) return actions.remove(path)
            update({
                include: [...new Set([...params.include, path])],
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
            })
        },
        remove: (path) => {
            refreshTags()
            update({
                tag: params.tag === path ? null : params.tag,
                include: without(params.include, path),
                exclude: without(params.exclude, path),
            })
        },
        edit: (path) => setEditTarget(path),
        newChild: (path) => setNewTagParent(path),
        setOrder: (path, order) => {
            const key = path ?? ''
            write({tag_path: key, children_order: order})
            if (order !== 'manual' && reorderUnder === key) setReorderUnder(null)
        },
        setOrderDesc: (path, desc) => {
            const key = path ?? ''
            write({tag_path: key, children_order_desc: desc})
            // Dragging along a list that just reversed would move rows the wrong way.
            if (reorderUnder === key) setReorderUnder(null)
        },
        // Moving a row under a parent sorted by anything but ascending `manual` is meaningless, so
        // entering reorder mode switches it and says so (§7).
        reorder: (path) => {
            const key = path ?? ''
            if (reorderUnder === key) return setReorderUnder(null)
            const meta = metaByPath.get(key)
            const order = meta?.children_order ?? 'manual'
            const desc = meta?.children_order_desc ?? false
            if (order !== 'manual' || desc) {
                write({tag_path: key, children_order: 'manual', children_order_desc: false})
                toast.info('Subtags now sort manually')
            }
            // Start from an explicitly numbered list, or the first move down is a no-op — an unset
            // `sort_index` sorts last, so the one row that gains an index jumps to the front.
            for (const w of initialOrderWrites(childrenOf(key), desc)) write(w)
            setReorderUnder(key)
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
    const noFilter = !params.tag && !params.include.length && !params.exclude.length
    const reorderName = reorderUnder ? nameOf(reorderUnder) : 'top-level tags'
    const rootMeta = metaByPath.get('')
    const rootOrder = rootMeta?.children_order ?? 'manual'
    const rootDesc = rootMeta?.children_order_desc ?? false

    return (
        <div className="flex h-full flex-col">
            <button
                onClick={() => update({tag: null, include: [], exclude: [], hierarchy: null, hpath: ''})}
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
                {reorderUnder === null && (
                    <DropdownMenu>
                        <DropdownMenuTrigger
                            className="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-muted hover:text-foreground">
                            {rootDesc ? <ArrowDown className="h-3 w-3"/> : <ArrowUp className="h-3 w-3"/>}
                            {ORDERS.find((o) => o.value === rootOrder)?.label ?? 'Order'}
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-44">
                            <OrderChoices path={null} current={rootOrder} desc={rootDesc} actions={actions}/>
                        </DropdownMenuContent>
                    </DropdownMenu>
                )}
            </div>

            {/* Reorder mode is entered from a node's `…` menu, which is not where anyone looks to
                leave it — so it gets one unmissable exit that covers every level. */}
            {reorderUnder !== null && (
                <div className="mx-2 mt-1 flex items-center gap-2 rounded-md border border-primary/40 bg-primary/10 px-2 py-1 text-[11px] text-primary">
                    <ArrowDownUp className="h-3 w-3 shrink-0"/>
                    <span className="min-w-0 truncate">Reordering {reorderName}</span>
                    <button
                        className="ml-auto flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 font-medium hover:bg-primary/15"
                        onClick={() => actions.reorder(reorderUnder || null)}
                    >
                        <Check className="h-3 w-3"/>
                        Done
                    </button>
                </div>
            )}

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
                lockedTag={shareTag ?? undefined}
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
