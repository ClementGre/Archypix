import {type MouseEvent, useEffect, useMemo, useRef} from 'react'
import {useSearchParams} from 'react-router-dom'
import {AlertCircle, ImageOff, Loader2} from 'lucide-react'
import {usePictures} from '@/hooks/usePictures'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {apiErrorMessage} from '@/api/client'
import {PhotoCard} from './PhotoCard'
import {Lightbox} from './Lightbox'

export function PhotoGrid() {
    const {filters, params} = useGalleryParams()
    const rowHeight = useUIStore((s) => s.rowHeight)
    const [, setSp] = useSearchParams()
    const {data, isPending, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage} = usePictures(filters)

    const items = useMemo(() => {
        const all = data?.pages.flatMap((p) => p.items) ?? []
        const q = params.q.trim().toLowerCase()
        return q ? all.filter((it) => (it.filename ?? '').toLowerCase().includes(q)) : all
    }, [data, params.q])

    const orderedIds = useMemo(() => items.map((i) => i.id), [items])

    const selected = useSelectionStore((s) => s.selected)
    const select = useSelectionStore((s) => s.select)
    const toggle = useSelectionStore((s) => s.toggle)
    const selectTo = useSelectionStore((s) => s.selectTo)
    const clear = useSelectionStore((s) => s.clear)

    const handleSelect = (id: string) => (e: MouseEvent) => {
        e.stopPropagation()
        if (e.metaKey || e.ctrlKey) toggle(id)
        else if (e.shiftKey) selectTo(id, orderedIds)
        else if (selected.length === 1 && selected[0] === id) clear()
        else select(id)
    }

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

    if (isPending) {
        return (
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
    }

    if (isError) {
        return (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                <AlertCircle className="h-8 w-8"/>
                <p>Could not load photos.</p>
                <p className="text-xs">{apiErrorMessage(error)}</p>
            </div>
        )
    }

    if (!items.length) {
        return (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center text-sm text-muted-foreground">
                <ImageOff className="h-8 w-8"/>
                <p>No photos match this view.</p>
            </div>
        )
    }

    return (
        <div className="h-full overflow-y-auto p-3" onMouseDown={(e) => e.target === e.currentTarget && clear()}>
            <ul className="m-0 flex list-none flex-wrap content-start gap-1.5 p-0">
                {items.map((it) => (
                    <PhotoCard
                        key={it.id}
                        item={it}
                        rowHeight={rowHeight}
                        selected={selected.includes(it.id)}
                        onSelect={handleSelect(it.id)}
                        onOpen={() => openViewer(it.id)}
                    />
                ))}
                {/* Absorbs trailing space so the last row keeps natural sizing. */}
                <li aria-hidden className="h-0" style={{flexGrow: 1e7, flexBasis: 0}}/>
            </ul>
            <div ref={sentinel} className="flex h-12 items-center justify-center">
                {isFetchingNextPage && <Loader2 className="h-5 w-5 animate-spin text-muted-foreground"/>}
            </div>

            <Lightbox items={items}/>
        </div>
    )
}
