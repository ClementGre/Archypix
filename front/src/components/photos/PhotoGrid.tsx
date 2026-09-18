import {Fragment, useCallback, useEffect, useMemo, useRef, useState} from 'react'
import {useSearchParams} from 'react-router-dom'
import {AlertCircle, ChevronRight, FolderOpen, ImageOff, Loader2, Wrench} from 'lucide-react'
import {useHierarchies, useHierarchyBrowse} from '@/hooks/useHierarchies'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useTimelineView} from '@/hooks/useTimelineView'
import {useBatchEditTags, useTagTree} from '@/hooks/useTags'
import {useSelectionStore} from '@/stores/selection'
import {areSiblings, useTagDragStore} from '@/stores/tagDrag'
import {useFixReference} from '@/stores/fixReference'
import {useGridItems} from '@/stores/gridItems'
import {useTimelineExpansion} from '@/stores/timelineExpansion'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import {display, type TagNode, type TrashView} from '@/lib/tagTree'
import {NO_GROUPING} from '@/lib/grouping'
import type {PictureFilters} from '@/lib/types'
import {TagFilterBar} from '@/components/tags/TagFilterBar'
import {TagDropDialog, type TagDrop, undoAction} from '@/components/tags/TagDropDialog'
import {GroupedGridProvider, useGroupedGrid, useRegisterSection} from './grouped/GroupedGridContext'
import {PhotoCards, useGridVariant} from './grouped/PhotoCards'
import {TagStream} from './grouped/TagStream'
import {Lightbox} from './Lightbox'
import {TrashToggle} from './TrashToggle'
import {IssuesFilter} from './IssuesFilter'
import {ScopeToggle} from './ScopeToggle'
import {SortMenu} from './SortMenu'
import {ViewMenu} from './ViewMenu'
import {DateFilter} from './DateFilter'
import {SelectionActionBar} from './batch/SelectionActionBar'
import {ReferenceBar} from './fix/ReferenceBar'
import {toast} from 'sonner'

/** Breadcrumb for the active hierarchy directory; segments are clickable. */
function HierarchyBreadcrumb() {
    const {params, update} = useGalleryParams()
    const {data: hierarchies} = useHierarchies()
    const name = hierarchies?.find((h) => h.id === params.hierarchy)?.name ?? 'Hierarchy'
    const segments = params.hpath ? params.hpath.split('/') : []

    return (
        <div className="flex flex-wrap items-center gap-1 text-sm">
            <FolderOpen className="mr-1 h-4 w-4 shrink-0 text-muted-foreground"/>
            <button
                onClick={() => update({hpath: ''})}
                className={cn('rounded px-1 hover:bg-muted', !params.hpath ? 'font-medium text-foreground' : 'text-muted-foreground')}
            >
                {name}
            </button>
            {segments.map((seg, i) => {
                const path = segments.slice(0, i + 1).join('/')
                const isLast = i === segments.length - 1
                return (
                    <Fragment key={path}>
                        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60"/>
                        <button
                            onClick={() => update({hpath: path})}
                            className={cn('rounded px-1 hover:bg-muted', isLast ? 'font-medium text-foreground' : 'text-muted-foreground')}
                        >
                            {seg}
                        </button>
                    </Fragment>
                )
            })}
        </div>
    )
}

/** Hierarchy browse keeps today's flat rendering — it has its own directory structure (§10.9). It
 *  registers as a single section so selection, the lightbox and paging stay on one code path. */
function HierarchyStream({filters}: { filters: PictureFilters }) {
    const {params} = useGalleryParams()
    const variant = useGridVariant()
    const q = useHierarchyBrowse(params.hierarchy, params.hpath, filters, {enabled: true, variant})
    const items = useMemo(() => {
        const flat = q.data?.pages.flatMap((p) => p.items) ?? []
        const seen = new Set<string>()
        return flat.filter((it) => (seen.has(it.id) ? false : (seen.add(it.id), true)))
    }, [q.data])

    useRegisterSection('hierarchy', {
        order: [0],
        items,
        fetchNextPage: q.fetchNextPage,
        hasNextPage: !!q.hasNextPage,
    })

    const sentinel = useRef<HTMLDivElement>(null)
    useEffect(() => {
        const el = sentinel.current
        if (!el) return
        const io = new IntersectionObserver((entries) => {
            if (entries[0]?.isIntersecting && q.hasNextPage && !q.isFetchingNextPage) q.fetchNextPage()
        })
        io.observe(el)
        return () => io.disconnect()
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [q.hasNextPage, q.isFetchingNextPage])

    if (q.isError) {
        return (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                <AlertCircle className="h-8 w-8"/>
                <p>Could not load photos.</p>
                <p className="text-xs">{apiErrorMessage(q.error)}</p>
            </div>
        )
    }
    if (!q.isPending && !items.length) {
        return (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                <ImageOff className="h-8 w-8"/>
                <p>This directory has no photos.</p>
            </div>
        )
    }

    return (
        <>
            <ul className="m-0 flex list-none flex-wrap content-start gap-1.5 p-0 select-none">
                <PhotoCards items={items} sourceTag={null}/>
                <li aria-hidden className="h-0" style={{flexGrow: 1e7, flexBasis: 0}}/>
            </ul>
            <div ref={sentinel} className="flex h-12 items-center justify-center">
                {q.isFetchingNextPage && <Loader2 className="h-5 w-5 animate-spin text-muted-foreground"/>}
            </div>
        </>
    )
}

/**
 * Selection plumbing shared by every stream: it reads the **flattened** visible order from the
 * context rather than any one section's slice (§8), so shift-click spans groups and the lightbox
 * continues from the end of one group into the next.
 */
function GridPlumbing() {
    const {params} = useGalleryParams()
    const [sp] = useSearchParams()
    const {selectionFilter} = useTimelineView()
    const {items, loadMore} = useGroupedGrid()
    const variant = useGridVariant()
    const referenceActive = useFixReference((s) => s.active)

    const setSelection = useSelectionStore((s) => s.setSelection)
    const selectAll = useSelectionStore((s) => s.selectAll)
    const queueLand = useSelectionStore((s) => s.queueLand)
    const pendingLand = useSelectionStore((s) => s.pendingLand)
    const clear = useSelectionStore((s) => s.clear)

    // Publish the loaded, sorted grid so the fix panels can scan for grid-local anchors (30 §5.2).
    const setGridItems = useGridItems((s) => s.setItems)
    useEffect(() => setGridItems(items), [items, setGridItems])

    // ⌘/Ctrl+A selects everything matching the current view, unless focus is in a field.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'a') return
            const t = e.target as HTMLElement | null
            if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
            // In the reference-picking phase, ⌘A would hijack the reference set — ignore it.
            if (useFixReference.getState().active) return
            e.preventDefault()
            selectAll(selectionFilter)
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [selectionFilter])

    // Any view change clears the selection: a select-all's membership would no longer match. Toggling
    // `subtag` ↔ `all` shares a filter, so it correctly preserves the selection (§7).
    // Exception: the fix tools queued a "land here" intent — keep it through the clear; the consume
    // effect below resolves it once the restored grid has loaded.
    const filterSig = JSON.stringify(selectionFilter)
    useEffect(() => {
        if (useSelectionStore.getState().pendingLand !== null) return
        clear()
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [filterSig])

    // Resolve a queued land intent (Apply / Skip / Cancel returning from the reference phase).
    //
    // Timing is the whole game here. Exiting the phase flips the reference store out (a Zustand
    // update) but restores the URL through React Router — these land in **separate commits**. So in
    // the first commit `referenceActive` is already false while the view is still the reference grid;
    // resolving then would pick against the wrong grid and, worse, null `pendingLand` so the *next*
    // commit's view-change clear wipes the fresh selection. We therefore wait until the on-screen
    // view actually matches the intent's `destSig`. `destSig === null` means no restore is pending.
    useEffect(() => {
        if (!pendingLand || referenceActive) return
        const {anchorId, advance, destSig} = pendingLand
        if (destSig != null && destSig !== filterSig) return
        if (advance && params.fix) {
            if (!items.length) return
            const idx = items.findIndex((i) => i.id === anchorId)
            const rest = idx === -1 ? items : items.slice(idx + 1)
            const next = rest.find((i) => !i.deleted_at && (params.fix === 'gps' ? !i.has_gps : !i.captured_at))
            if (!next) toast.info('No more pictures to land on.')
            setSelection([next ? next.id : anchorId])
        } else {
            setSelection([anchorId])
        }
        queueLand(null)
    }, [pendingLand, referenceActive, filterSig, items, params.fix, setSelection, queueLand])

    // Page the section the viewer is standing in, not whichever is unfinished first (§8).
    const viewId = sp.get('view')
    return <Lightbox items={items} gridVariant={variant} loadMore={() => loadMore(viewId)}/>
}

export function PhotoGrid() {
    const {filters, params} = useGalleryParams()
    const view = useTimelineView()
    const {tree, metaByPath} = useTagTree(params.trash as TrashView)
    const isBrowsing = !!params.hierarchy
    const referenceActive = useFixReference((s) => s.active)
    const clear = useSelectionStore((s) => s.clear)
    const drag = useTagDragStore()
    const edit = useBatchEditTags()
    const [drop, setDrop] = useState<TagDrop | null>(null)

    // Counts come from the unfiltered tag payload, so a cross-cutting filter makes them wrong (§5).
    const hideCounts = params.include.length > 0 || params.exclude.length > 0

    const rootNode = useMemo(
        () => (view.root ? findNode(tree, view.root) : null),
        [tree, view.root],
    )

    // ⌘A selects the subtree, not the visible set (§7) — `include: [T]` covers photos inside blocks
    // that are not on screen, so the selection bar says how many groups that is.
    const overrides = useTimelineExpansion((s) => s.overrides)
    const collapsedGroups =
        view.mode === 'subtag'
            ? (rootNode ? rootNode.children : tree).filter((c) => !overrides[c.path]).length
            : 0

    // Dropping photos on a subtag block is the same action as dropping on a tree row (34 §9) — but
    // here the drag started *inside* a tag, so "sibling" is defined and the dialog can offer to
    // remove the source tag.
    const onDropOnTag = useCallback(
        (target: TagNode) => {
            if (!drag.selection) return
            const payload: TagDrop = {
                selection: drag.selection,
                count: drag.count,
                sourceTag: drag.sourceTag,
                targetTag: target.path,
                targetName: target.name,
                sourceName: drag.sourceTag ? display(drag.sourceTag, metaByPath.get(drag.sourceTag)) : null,
            }
            drag.end()
            const silent =
                payload.count === 1 &&
                !(payload.sourceTag && areSiblings(payload.sourceTag, payload.targetTag))
            if (!silent) return setDrop(payload)
            edit.mutate(
                {selection: payload.selection, add_tags: [payload.targetTag]},
                {
                    onSuccess: () => toast.success(`Added to ${payload.targetName}`, undoAction(payload, null, edit)),
                    onError: (e) => toast.error(apiErrorMessage(e)),
                },
            )
        },
        [drag, edit, metaByPath],
    )

    // The padding is *inside* the scroll container, not on it: a sticky header offset from the top of a
    // padded scrollport leaves a strip of scrolling photos above it. Here the padding scrolls away
    // instead, and headers bleed back out of it horizontally (`PAD_ROOT`). `min-h-full` keeps the
    // click-to-clear target covering the whole viewport even when the grid is short.
    const body = (
        <div className="md:h-full md:overflow-y-auto">
            <div className="min-h-full p-3" onMouseDown={(e) => e.target === e.currentTarget && clear()}>
                {isBrowsing ? (
                    <HierarchyStream filters={filters}/>
                ) : (
                    <TagStream
                        path={view.root}
                        node={rootNode}
                        order={[0]}
                        trash={params.trash as TrashView}
                        bucketCtx={view.bucketCtx}
                        base={filters}
                        hideCounts={hideCounts}
                        onDropOnTag={onDropOnTag}
                        modeOverride={view.pinned ? 'all' : undefined}
                        groupingOverride={view.pinned ? NO_GROUPING : undefined}
                    />
                )}
            </div>
            <GridPlumbing/>
        </div>
    )

    // A single header row tops the grid: on the left the active hierarchy breadcrumb, or (flat view)
    // the tag-filter breadcrumb of active include/exclude chips; on the right the view controls.
    return (
        <>
            <div className="flex h-full min-h-0 flex-col">
                {/* `mr-auto` on the breadcrumb (content-sized, not flex-1) pushes the control cluster
                    right and lets it wrap to the next line when a long breadcrumb leaves no room. */}
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-border px-3 py-1.5">
                    <div className="mr-auto min-w-0 max-w-full">
                        {isBrowsing ? <HierarchyBreadcrumb/> : <TagFilterBar/>}
                    </div>
                    <div className="flex flex-wrap items-center gap-1.5">
                        <DateFilter/>
                        <IssuesFilter/>
                        <ScopeToggle/>
                        <TrashToggle/>
                        <ViewMenu/>
                        <SortMenu/>
                    </div>
                </div>
                {/* Both overrides are stated with their reason and restore on leaving fix mode —
                    neither is written to `tag_metadata` (§9). */}
                {view.pinned && (
                    <p className="flex items-center gap-1.5 border-b border-border bg-muted/40 px-3 py-1 text-[11px] text-muted-foreground">
                        <Wrench className="h-3 w-3 shrink-0"/>
                        Fix mode needs a flat stream to find temporal and spatial neighbours, so grouping and
                        subtag blocks are off. Both come back when you leave it.
                    </p>
                )}
                <div className="min-h-0 flex-1">
                    <GroupedGridProvider>{body}</GroupedGridProvider>
                </div>
            </div>
            <TagDropDialog drop={drop} onClose={() => setDrop(null)}/>
            {referenceActive ? <ReferenceBar/> : <SelectionActionBar selectionFilter={view.selectionFilter} collapsedGroups={collapsedGroups}/>}
        </>
    )
}

function findNode(nodes: TagNode[], path: string): TagNode | null {
    for (const n of nodes) {
        if (n.path === path) return n
        if (path.startsWith(`${n.path}.`)) return findNode(n.children, path)
    }
    return null
}
