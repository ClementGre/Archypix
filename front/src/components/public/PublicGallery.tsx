import {type MouseEvent, useCallback, useEffect, useMemo, useRef} from 'react'
import {useSearchParams} from 'react-router-dom'
import {useInfiniteQuery} from '@tanstack/react-query'
import {Loader2} from 'lucide-react'
import {listPublicPictures} from '@/api/publicShares'
import {apiErrorMessage} from '@/api/client'
import {variantForSize} from '@/lib/utils'
import {isMemberSelected, useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {PhotoCard} from '@/components/photos/PhotoCard'
import {Lightbox} from '@/components/photos/Lightbox'
import {SelectionActionBar} from '@/components/photos/batch/SelectionActionBar'
import {usePublicShare} from '@/components/public/context'

export function PublicGallery() {
    const {backendUrl, token, session} = usePublicShare()
    const [, setSp] = useSearchParams()
    const rowHeight = useUIStore((s) => s.rowHeight)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const isMobile = useIsMobile()
    const variant = variantForSize(rowHeight)

    const query = useInfiniteQuery({
        queryKey: ['publicPictures', backendUrl, token, variant],
        initialPageParam: 1,
        queryFn: ({pageParam}) =>
            listPublicPictures(backendUrl, token, {page: pageParam as number, thumbnail: variant, sessionJwt: session}),
        getNextPageParam: (last) => (last.page * last.page_size < last.total ? last.page + 1 : undefined),
        retry: false,
    })

    const items = useMemo(() => {
        const flat = query.data?.pages.flatMap((p) => p.items) ?? []
        const seen = new Set<string>()
        return flat.filter((i) => (seen.has(i.id) ? false : (seen.add(i.id), true)))
    }, [query.data])
    const orderedIds = useMemo(() => items.map((i) => i.id), [items])

    const {fetchNextPage, hasNextPage, isFetchingNextPage} = query
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
        const obs = new IntersectionObserver((entries) => {
            if (entries[0].isIntersecting && hasNextPage && !isFetchingNextPage) fetchNextPage()
        })
        obs.observe(el)
        return () => obs.disconnect()
    }, [fetchNextPage, hasNextPage, isFetchingNextPage])

    // Same selection model as the authenticated gallery (the feature-14 descriptor store), but purely
    // explicit — a token-gated album has no server-side `PictureFilter`, so select-all/invert operate
    // on the loaded ids.
    const selQuery = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const multiSelect = useSelectionStore((s) => s.multiSelect)
    const select = useSelectionStore((s) => s.select)
    const toggle = useSelectionStore((s) => s.toggle)
    const selectTo = useSelectionStore((s) => s.selectTo)
    const enterMultiSelect = useSelectionStore((s) => s.enterMultiSelect)
    const setSelection = useSelectionStore((s) => s.setSelection)
    const clear = useSelectionStore((s) => s.clear)

    // ⌘/Ctrl+A selects every loaded picture (explicit), unless focus is in a field.
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== 'a') return
            const t = e.target as HTMLElement | null
            if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
            e.preventDefault()
            setSelection(orderedIds)
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [orderedIds, setSelection])

    const handleSelect = (id: string) => (e: MouseEvent) => {
        e.stopPropagation()
        if (e.metaKey || e.ctrlKey) toggle(id)
        else if (e.shiftKey) selectTo(id, orderedIds)
        else if (multiSelect) toggle(id)
        else if (selQuery === null && includeIds.length === 1 && includeIds[0] === id) clear()
        else {
            select(id)
            if (isMobile) openMobileDrawer('right')
        }
    }
    const handleLongPress = (id: string) => () => {
        if (multiSelect) toggle(id)
        else enterMultiSelect(id)
    }

    if (query.isPending) {
        return <Center><Loader2 className="h-6 w-6 animate-spin text-muted-foreground"/></Center>
    }
    if (query.isError) {
        return <Center><p className="text-sm text-destructive">{apiErrorMessage(query.error)}</p></Center>
    }
    if (items.length === 0) {
        return <Center><p className="text-sm text-muted-foreground">This album has no pictures yet.</p></Center>
    }

    return (
        <>
            <div className="h-full overflow-y-auto p-3" onMouseDown={(e) => e.target === e.currentTarget && clear()}>
                {/* Justified grid — same flex layout + PhotoCard as the authenticated gallery. */}
                <ul className="m-0 flex list-none flex-wrap content-start gap-1.5 p-0 select-none">
                    {items.map((it) => (
                        <PhotoCard
                            key={it.id}
                            item={it}
                            rowHeight={rowHeight}
                            selected={isMemberSelected(selQuery, includeIds, excludeIds, it.id)}
                            multiSelect={multiSelect}
                            onSelect={handleSelect(it.id)}
                            onLongPress={handleLongPress(it.id)}
                            onOpen={() => openViewer(it.id)}
                        />
                    ))}
                    <li aria-hidden className="h-0" style={{flexGrow: 1e7, flexBasis: 0}}/>
                </ul>
                <div ref={sentinel} className="flex h-12 items-center justify-center">
                    {isFetchingNextPage && <Loader2 className="h-5 w-5 animate-spin text-muted-foreground"/>}
                </div>
                {/* Reuse the authenticated Lightbox (read-only via the public PictureSource in context). */}
                <Lightbox items={items} gridVariant={variant} loadMore={loadMore}/>
            </div>
            {/* The shared floating multi-select bar, in explicit mode over the loaded ids. */}
            <SelectionActionBar
                onSelectAll={() => setSelection(orderedIds)}
                onInvert={() => setSelection(orderedIds.filter((id) => !isMemberSelected(selQuery, includeIds, excludeIds, id)))}
            />
        </>
    )
}

function Center({children}: { children: React.ReactNode }) {
    return <div className="flex min-h-[40vh] items-center justify-center p-8">{children}</div>
}
