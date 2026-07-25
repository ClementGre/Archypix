import {Fragment, type MouseEvent, useCallback, useEffect, useMemo, useRef} from 'react'
import {useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {AlertCircle, ChevronRight, FolderOpen, ImageOff, Loader2} from 'lucide-react'
import {usePictures} from '@/hooks/usePictures'
import {getPicture} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {useHierarchies, useHierarchyBrowse} from '@/hooks/useHierarchies'
import {useSettings} from '@/hooks/useSettings'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {isMemberSelected, useSelectionStore} from '@/stores/selection'
import {useFixReference} from '@/stores/fixReference'
import {useFixHighlight} from '@/stores/fixHighlight'
import {useGridItems} from '@/stores/gridItems'
import {useUIStore} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {apiErrorMessage} from '@/api/client'
import {cn, variantForSize} from '@/lib/utils'
import {TagFilterBar} from '@/components/tags/TagFilterBar'
import {PhotoCard} from './PhotoCard'
import {Lightbox} from './Lightbox'
import {TrashToggle} from './TrashToggle'
import {IssuesFilter} from './IssuesFilter'
import {ScopeToggle} from './ScopeToggle'
import {SortMenu} from './SortMenu'
import {DateFilter} from './DateFilter'
import {SelectionActionBar} from './batch/SelectionActionBar'
import {ReferenceBar} from './fix/ReferenceBar'
import {toast} from "sonner";

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

export function PhotoGrid() {
    const {filters, params, selectionFilter} = useGalleryParams()
    const isBrowsing = !!params.hierarchy
    const trashOnly = params.trash === 'only'
    const {data: settings} = useSettings()
    const retentionDays = settings?.trash_retention_days ?? 30
    const rowHeight = useUIStore((s) => s.rowHeight)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const isMobile = useIsMobile()
    const [, setSp] = useSearchParams()

    // Fix-mode state: highlight props for the cards, and the reference-picking phase overrides card
    // selection to build the reference set (persistent across tag navigation, §7).
    const referenceActive = useFixReference((s) => s.active)
    const refIds = useFixReference((s) => s.refIds)
    const toggleRef = useFixReference((s) => s.toggleRef)
    const fixTargetIds = useFixReference((s) => s.targetIds)
    const anchorIds = useFixHighlight((s) => s.anchorIds)

    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)

    // The picture being fixed (selected single, or the stashed reference target). Its detail drives the
    // grid distance overlays: time proximity in GPS mode (client-side) and geo distance in date mode
    // (§3, requested from the server via `geoRef` so the grid isn't reordered).
    const fixTargetId = params.fix
        ? referenceActive
            ? fixTargetIds[0] ?? null
            : query === null && includeIds.length === 1 ? includeIds[0] : null
        : null
    const fixTargetDetail = useQuery({
        queryKey: queryKeys.picture(fixTargetId ?? ''),
        enabled: !!fixTargetId,
        queryFn: () => getPicture(fixTargetId!),
    }).data
    const fixRefTime = params.fix === 'gps' ? fixTargetDetail?.captured_at ?? null : null
    const fixGeoRef = params.fix === 'date' && fixTargetDetail?.gps_lat != null && fixTargetDetail?.gps_lng != null
        ? {lat: fixTargetDetail.gps_lat, lng: fixTargetDetail.gps_lng}
        : null

    // Request a thumbnail variant sized to the current zoom (row height).
    const variant = variantForSize(rowHeight)
    const picturesQ = usePictures(filters, {enabled: !isBrowsing, variant, geoRef: fixGeoRef})
    const browseQ = useHierarchyBrowse(params.hierarchy, params.hpath, filters, {enabled: isBrowsing, variant})
    const active = isBrowsing ? browseQ : picturesQ
    const {data, isPending, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage, isPlaceholderData} = active

    // Dedup by id: as new pictures shift pagination, consecutive pages can overlap and re-emit an
    // already-seen item, which would render a duplicate card (and, if selected, look doubly selected).
    const items = useMemo(() => {
        const flat = data?.pages.flatMap((p) => p.items) ?? []
        const seen = new Set<string>()
        return flat.filter((it) => (seen.has(it.id) ? false : (seen.add(it.id), true)))
    }, [data])

    const orderedIds = useMemo(() => items.map((i) => i.id), [items])

    // Publish the loaded, sorted grid so the fix panels can scan for grid-local anchors (feature 30 §5.2).
    const setGridItems = useGridItems((s) => s.setItems)
    useEffect(() => setGridItems(items), [items, setGridItems])

    const multiSelect = useSelectionStore((s) => s.multiSelect)
    const select = useSelectionStore((s) => s.select)
    const toggle = useSelectionStore((s) => s.toggle)
    const selectTo = useSelectionStore((s) => s.selectTo)
    const enterMultiSelect = useSelectionStore((s) => s.enterMultiSelect)
    const selectAll = useSelectionStore((s) => s.selectAll)
    const setSelection = useSelectionStore((s) => s.setSelection)
    const queueLand = useSelectionStore((s) => s.queueLand)
    const pendingLand = useSelectionStore((s) => s.pendingLand)
    const clear = useSelectionStore((s) => s.clear)

    // ⌘/Ctrl+A selects everything matching the current view (§2.1), unless focus is in a field.
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

    // Any view change (tag / scope / sort / hierarchy dir) clears the selection: a select-all's
    // membership would no longer match, and keeping an explicit selection across an unrelated view
    // is inconsistent. The cleared-on-mount run is a harmless no-op (selection already empty).
    // Exception: the fix tools queued a "land here" intent (Apply/Skip restoring the pre-reference
    // view) — keep it through the clear; the consume effect below resolves it once the restored grid
    // has loaded. This effect is declared first so it runs before that one.
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
    // commit's view-change clear wipes the fresh selection (the empty sidebar). We therefore wait
    // until the on-screen view actually matches the intent's `destSig` (the signature of the view
    // captured when picking began). `destSig === null` means no restore is pending (a plain apply
    // without reference picking), so resolve in place. Only advancing also needs the restored grid's
    // real data (`!isPlaceholderData`) to find the next still-missing picture.
    useEffect(() => {
        if (!pendingLand || referenceActive) return
        const {anchorId, advance, destSig} = pendingLand
        if (destSig != null && destSig !== filterSig) return
        if (advance && params.fix) {
            if (isPlaceholderData) return
            const idx = items.findIndex((i) => i.id === anchorId)
            const rest = idx === -1 ? items : items.slice(idx + 1)
            const next = rest.find((i) => !i.deleted_at && (params.fix === 'gps' ? !i.has_gps : !i.captured_at))
            if (!next) {
                toast.info('No more pictures to land on.')
            }
            setSelection([next ? next.id : anchorId])
        } else {
            setSelection([anchorId])
        }
        queueLand(null)
    }, [pendingLand, referenceActive, filterSig, isPlaceholderData, items, params.fix, setSelection, queueLand])

    const handleSelect = (id: string) => (e: MouseEvent) => {
        e.stopPropagation()
        if (e.metaKey || e.ctrlKey) toggle(id)
        else if (e.shiftKey) selectTo(id, orderedIds)
        else if (multiSelect) toggle(id)
        else if (query === null && includeIds.length === 1 && includeIds[0] === id) clear()
        else {
            select(id)
            // On mobile a single tap surfaces the details/selection drawer.
            if (isMobile) openMobileDrawer('right')
        }
    }

    // Long-press (touch): start multi-select, or extend it if already active.
    const handleLongPress = (id: string) => () => {
        if (multiSelect) toggle(id)
        else enterMultiSelect(id)
    }

    // Let the Lightbox page in more items as it nears the end of what's loaded (large libraries).
    const loadMore = useCallback(() => {
        if (hasNextPage && !isFetchingNextPage) fetchNextPage()
    }, [hasNextPage, isFetchingNextPage, fetchNextPage])

    const openViewer = (id: string) =>
        setSp((prev) => {
            const next = new URLSearchParams(prev)
            next.set('view', id)
            return next
        })

    const sentinel = useRef<HTMLDivElement>(null)
    useEffect(() => {
        const el = sentinel.current
        if (!el) return
        const io = new IntersectionObserver((entries) => {
            if (entries[0]?.isIntersecting && hasNextPage && !isFetchingNextPage) fetchNextPage()
        })
        io.observe(el)
        return () => io.disconnect()
    }, [hasNextPage, isFetchingNextPage, fetchNextPage])

    let body
    if (isPending) {
        body = (
            <div className="flex flex-wrap content-start gap-1.5 p-3">
                {Array.from({length: 18}).map((_, i) => (
                    <div
                        key={i}
                        className="animate-pulse rounded-[3px] bg-muted"
                        style={{height: rowHeight, flexBasis: `${rowHeight * 1.4}px`, flexGrow: rowHeight * 1.4}}
                    />
                ))}
            </div>
        )
    } else if (isError) {
        body = (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                <AlertCircle className="h-8 w-8"/>
                <p>Could not load photos.</p>
                <p className="text-xs">{apiErrorMessage(error)}</p>
            </div>
        )
    } else if (!items.length) {
        body = (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                <ImageOff className="h-8 w-8"/>
                <p>{isBrowsing ? 'This directory has no photos.' : 'No photos match this view.'}</p>
            </div>
        )
    } else {
        body = (
            <div className="p-3 md:h-full md:overflow-y-auto" onMouseDown={(e) => e.target === e.currentTarget && clear()}>
                {/* select-none: shift-click range selection otherwise highlights the cards as text. */}
                <ul className="m-0 flex list-none flex-wrap content-start gap-1.5 p-0 select-none">
                    {items.map((it) => {
                        // While picking references, only photos that HAVE the field being interpolated
                        // can be a reference; the rest are dimmed + inert.
                        const canRef = params.fix === 'gps' ? it.has_gps : params.fix === 'date' ? !!it.captured_at : false
                        const refDisabled = referenceActive && !canRef
                        return (
                            <PhotoCard
                                key={it.id}
                                item={it}
                                rowHeight={rowHeight}
                                // In the reference phase a picked reference gets the interpolation-source
                                // (sky ring + blue check) style, the same as an automatic anchor — not the
                                // primary selection ring; a click toggles it (§7).
                                selected={referenceActive ? false : isMemberSelected(query, includeIds, excludeIds, it.id)}
                                multiSelect={multiSelect}
                                showPurgeCountdown={trashOnly}
                                retentionDays={retentionDays}
                                proximityRefTime={params.sort === 'time_near' ? params.nearTime : fixRefTime}
                                // No "missing" highlight while picking references — the context is choosing
                                // sources, not fixing; non-eligible photos are dimmed instead.
                                fixMode={referenceActive ? null : params.fix}
                                dimmed={refDisabled}
                                anchorRole={
                                    referenceActive
                                        ? refIds.includes(it.id) ? params.fix : null
                                        : params.fix === 'gps' && anchorIds.includes(it.id) ? 'gps' : null
                                }
                                onSelect={
                                    referenceActive
                                        ? (e) => {
                                            e.stopPropagation();
                                            if (canRef) toggleRef(it.id)
                                        }
                                        : handleSelect(it.id)
                                }
                                onLongPress={referenceActive ? () => {
                                    if (canRef) toggleRef(it.id)
                                } : handleLongPress(it.id)}
                                onOpen={() => openViewer(it.id)}
                            />
                        )
                    })}
                    {/* Absorbs trailing space so the last row keeps natural sizing. */}
                    <li aria-hidden className="h-0" style={{flexGrow: 1e7, flexBasis: 0}}/>
                </ul>
                <div ref={sentinel} className="flex h-12 items-center justify-center">
                    {isFetchingNextPage && <Loader2 className="h-5 w-5 animate-spin text-muted-foreground"/>}
                </div>

                <Lightbox items={items} gridVariant={variant} loadMore={loadMore}/>
            </div>
        )
    }

    // A single header row tops the grid: on the left the active hierarchy breadcrumb, or (flat view)
    // the tag-filter breadcrumb of active include/exact/exclude chips (empty when none); on the right
    // the three-state trash toggle — the trash is a filter over this view, not a separate page.
    const content = (
        <div className="flex h-full min-h-0 flex-col">
            {/* Unified filter/sort toolbar. `mr-auto` on the breadcrumb (content-sized, not flex-1)
                pushes the control cluster right and — crucially — lets it wrap to the next line when a
                long breadcrumb leaves no room, instead of the cluster overflowing. The breadcrumb keeps
                `min-w-0` so its own chips wrap rather than forcing horizontal overflow. */}
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-border px-3 py-1.5">
                <div className="mr-auto min-w-0 max-w-full">
                    {isBrowsing ? <HierarchyBreadcrumb/> : <TagFilterBar/>}
                </div>
                <div className="flex flex-wrap items-center gap-1.5">
                    <DateFilter/>
                    <IssuesFilter/>
                    <ScopeToggle/>
                    <TrashToggle/>
                    <SortMenu/>
                </div>
            </div>
            <div className="min-h-0 flex-1">{body}</div>
        </div>
    )

    return (
        <>
            {content}
            {referenceActive ? <ReferenceBar/> : <SelectionActionBar/>}
        </>
    )
}
