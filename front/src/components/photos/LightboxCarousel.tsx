import {type RefObject, useEffect, useMemo, useRef, useState} from 'react'
import {useQuery} from '@tanstack/react-query'
import {getPictureUrl} from '@/api/pictures'
import {bestLoaded, recordImage, useImageCache} from '@/stores/imageCache'
import {cn, isVideoMime} from '@/lib/utils'
import type {PictureListItem, PictureVariant} from '@/lib/types'
import {displayDimensions, OrientedImage} from './OrientedImage'
import {FileTypeIcon} from './FileTypeIcon'
import {PlayBadge} from './PlayBadge'

/** Filmstrip thumbnail height (px). */
const THUMB_H = 60

/**
 * One filmstrip thumbnail. Reuses whatever variant the browser has already loaded (so the grid's
 * medium/large image is reused with no new presign), then the list `thumbnail_url` (unless it's the
 * heavy `large`), and only otherwise fetches a dedicated `small` presign. The image work is gated on
 * visibility (an `IntersectionObserver` on the strip) so a large library doesn't presign/load every
 * thumbnail up front — the box always renders (sized from the aspect ratio) to keep scrolling correct.
 */
function CarouselThumb({item, gridVariant, active, rootRef, onClick}: {
    item: PictureListItem
    gridVariant: PictureVariant
    active: boolean
    rootRef: RefObject<HTMLDivElement | null>
    onClick: () => void
}) {
    const btnRef = useRef<HTMLButtonElement>(null)
    const [inView, setInView] = useState(false)
    useEffect(() => {
        const el = btnRef.current
        if (!el || inView) return
        const io = new IntersectionObserver(
            (entries) => {
                if (entries[0]?.isIntersecting) {
                    setInView(true)
                    io.disconnect()
                }
            },
            {root: rootRef.current, rootMargin: '300px'},
        )
        io.observe(el)
        return () => io.disconnect()
    }, [inView, rootRef])

    const entry = useImageCache((s) => s.entries[item.id])
    const loaded = useMemo(() => bestLoaded(entry), [entry])

    const useListThumb = !loaded && gridVariant !== 'large' && !!item.thumbnail_url
    const needSmall = inView && !loaded && !useListThumb && !!item.thumbnail_url
    const {data: small} = useQuery({
        queryKey: ['pictures', 'url', item.id, 'small'],
        queryFn: () => getPictureUrl(item.id, 'small'),
        enabled: needSmall,
        staleTime: 10 * 60 * 1000,
    })

    const src = loaded?.url ?? (useListThumb ? item.thumbnail_url : small?.url) ?? null
    const variant: PictureVariant = loaded?.variant ?? (useListThumb ? gridVariant : 'small')

    const {width: dW, height: dH} = displayDimensions(item.width, item.height, item.orientation)
    const ratio = dW && dH ? dW / dH : 1

    return (
        <button
            ref={btnRef}
            data-id={item.id}
            onClick={onClick}
            aria-label={item.filename ?? 'photo'}
            className={cn(
                'group relative shrink-0 snap-center overflow-hidden rounded bg-checkerboard ring-1 ring-white/10 transition-all',
                active ? 'ring-2 ring-primary' : 'opacity-60 hover:opacity-100',
            )}
            style={{height: THUMB_H, width: THUMB_H * ratio}}
        >
            {inView && src ? (
                <>
                    <OrientedImage
                        src={src}
                        alt={item.filename ?? ''}
                        orientation={item.orientation}
                        width={item.width}
                        height={item.height}
                        loading="eager"
                        onLoad={() => recordImage(item.id, variant, src, true)}
                    />
                    {isVideoMime(item.mime_type) && <PlayBadge size="xs"/>}
                </>
            ) : inView ? (
                <span className="flex h-full w-full items-center justify-center text-white/50">
                    <FileTypeIcon mime={item.mime_type} filename={item.filename} className="h-6 w-6"/>
                </span>
            ) : null}
        </button>
    )
}

/**
 * Horizontal filmstrip at the bottom of the lightbox. The current picture is centred; clicking a
 * thumb — or sliding/scrolling the strip so a different thumb lands in the centre — changes it.
 */
export function LightboxCarousel({items, currentId, gridVariant = 'medium', onSelect}: {
    items: PictureListItem[]
    currentId: string
    gridVariant?: PictureVariant
    onSelect: (id: string) => void
}) {
    const scrollRef = useRef<HTMLDivElement>(null)
    // True while we are programmatically centring the current thumb — suppresses the scroll handler
    // so re-centring can't be mistaken for a user slide (which would fight the current selection).
    const programmatic = useRef(false)
    const timer = useRef<number | undefined>(undefined)

    // Centre the active thumb whenever the current picture changes (or the list grows, so a thumb
    // that was last when centred re-centres once trailing items are appended).
    useEffect(() => {
        const cont = scrollRef.current
        if (!cont) return
        const el = cont.querySelector<HTMLElement>(`[data-id="${currentId}"]`)
        if (!el) return
        programmatic.current = true
        cont.scrollTo({left: el.offsetLeft - (cont.clientWidth - el.clientWidth) / 2, behavior: 'smooth'})
        window.clearTimeout(timer.current)
        timer.current = window.setTimeout(() => (programmatic.current = false), 450)
    }, [currentId, items.length])

    // Slide-to-select: after a manual scroll settles, adopt the thumb nearest the centre.
    const onScroll = () => {
        if (programmatic.current) return
        window.clearTimeout(timer.current)
        timer.current = window.setTimeout(() => {
            const cont = scrollRef.current
            if (!cont) return
            const center = cont.scrollLeft + cont.clientWidth / 2
            let nearest: string | null = null
            let best = Infinity
            for (const child of Array.from(cont.children) as HTMLElement[]) {
                const id = child.dataset.id
                if (!id) continue
                const d = Math.abs(child.offsetLeft + child.clientWidth / 2 - center)
                if (d < best) {
                    best = d
                    nearest = id
                }
            }
            if (nearest && nearest !== currentId) onSelect(nearest)
        }, 120)
    }

    return (
        <div
            ref={scrollRef}
            onScroll={onScroll}
            onClick={(e) => e.stopPropagation()}
            className="flex snap-x items-center gap-1.5 overflow-x-auto scroll-smooth px-2 py-2"
            style={{scrollbarWidth: 'thin'}}
        >
            {/* Half-width spacers let *any* thumb — first or last — reach the centre. */}
            <div aria-hidden className="shrink-0" style={{width: '50%', height: THUMB_H}}/>
            {items.map((it) => (
                <CarouselThumb
                    key={it.id}
                    item={it}
                    gridVariant={gridVariant}
                    active={it.id === currentId}
                    rootRef={scrollRef}
                    onClick={() => onSelect(it.id)}
                />
            ))}
            <div aria-hidden className="shrink-0" style={{width: '50%', height: THUMB_H}}/>
        </div>
    )
}
