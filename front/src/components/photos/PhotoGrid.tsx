import {Fragment, type MouseEvent, useCallback, useEffect, useMemo, useRef} from 'react'
import {useSearchParams} from 'react-router-dom'
import {AlertCircle, ChevronRight, FolderOpen, ImageOff, Loader2} from 'lucide-react'
import {usePictures} from '@/hooks/usePictures'
import {useHierarchies, useHierarchyBrowse} from '@/hooks/useHierarchies'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {isMemberSelected, useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {apiErrorMessage} from '@/api/client'
import {cn, variantForSize} from '@/lib/utils'
import {TagFilterBar} from '@/components/tags/TagFilterBar'
import {PhotoCard} from './PhotoCard'
import {Lightbox} from './Lightbox'
import {SelectionActionBar} from './batch/SelectionActionBar'

/** Breadcrumb for the active hierarchy directory; segments are clickable. */
function HierarchyBreadcrumb() {
    const {params, update} = useGalleryParams()
    const {data: hierarchies} = useHierarchies()
    const name = hierarchies?.find((h) => h.id === params.hierarchy)?.name ?? 'Hierarchy'
    const segments = params.hpath ? params.hpath.split('/') : []

    return (
        <div className="flex flex-wrap items-center gap-1 border-b border-border px-3 py-2 text-sm">
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
    const rowHeight = useUIStore((s) => s.rowHeight)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const isMobile = useIsMobile()
    const [, setSp] = useSearchParams()

    // Request a thumbnail variant sized to the current zoom (row height).
    const variant = variantForSize(rowHeight)
    const picturesQ = usePictures(filters, {enabled: !isBrowsing, variant})
    const browseQ = useHierarchyBrowse(params.hierarchy, params.hpath, filters, {enabled: isBrowsing, variant})
    const active = isBrowsing ? browseQ : picturesQ
    const {data, isPending, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage} = active

    // Dedup by id: as new pictures shift pagination, consecutive pages can overlap and re-emit an
    // already-seen item — which would render a duplicate card (and, if selected, look doubly selected).
    const items = useMemo(() => {
        const flat = data?.pages.flatMap((p) => p.items) ?? []
        const seen = new Set<string>()
        return flat.filter((it) => (seen.has(it.id) ? false : (seen.add(it.id), true)))
    }, [data])

    const orderedIds = useMemo(() => items.map((i) => i.id), [items])

    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const multiSelect = useSelectionStore((s) => s.multiSelect)
    const select = useSelectionStore((s) => s.select)
    const toggle = useSelectionStore((s) => s.toggle)
    const selectTo = useSelectionStore((s) => s.selectTo)
    const enterMultiSelect = useSelectionStore((s) => s.enterMultiSelect)
    const selectAll = useSelectionStore((s) => s.selectAll)
    const clear = useSelectionStore((s) => s.clear)

    // ⌘/Ctrl+A selects everything matching the current view (§2.1), unless focus is in a field.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'a') return
            const t = e.target as HTMLElement | null
            if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
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
    const filterSig = JSON.stringify(selectionFilter)
    useEffect(() => {
        clear()
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [filterSig])

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
            <div className="h-full overflow-y-auto p-3" onMouseDown={(e) => e.target === e.currentTarget && clear()}>
                {/* select-none: shift-click range selection otherwise highlights the cards as text. */}
                <ul className="m-0 flex list-none flex-wrap content-start gap-1.5 p-0 select-none">
                    {items.map((it) => (
                        <PhotoCard
                            key={it.id}
                            item={it}
                            rowHeight={rowHeight}
                            selected={isMemberSelected(query, includeIds, excludeIds, it.id)}
                            multiSelect={multiSelect}
                            onSelect={handleSelect(it.id)}
                            onLongPress={handleLongPress(it.id)}
                            onOpen={() => openViewer(it.id)}
                        />
                    ))}
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

    const content = isBrowsing ? (
        <div className="flex h-full min-h-0 flex-col">
            <HierarchyBreadcrumb/>
            <div className="min-h-0 flex-1">{body}</div>
        </div>
    ) : (
        // Flat view: a tag-filter breadcrumb (active include/exact/exclude) tops the grid, mirroring
        // the hierarchy breadcrumb. It renders nothing when no tag filter is active.
        <div className="flex h-full min-h-0 flex-col">
            <TagFilterBar/>
            <div className="min-h-0 flex-1">{body}</div>
        </div>
    )

    return (
        <>
            {content}
            <SelectionActionBar/>
        </>
    )
}
