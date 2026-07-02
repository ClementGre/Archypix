import {type ReactNode, useCallback, useEffect, useMemo, useRef, useState} from 'react'
import {createPortal} from 'react-dom'
import {useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {toast} from 'sonner'
import {
    ArchiveRestore,
    ChevronLeft,
    ChevronRight,
    Copy,
    Download,
    Images,
    Loader2,
    Maximize2,
    Minimize2,
    PanelTop,
    RotateCcw,
    RotateCw,
    Sparkles,
    Trash2,
    X,
} from 'lucide-react'
import type {PictureDetail, PictureListItem, PictureVariant} from '@/lib/types'
import {downloadOriginal, getPicture, getPictureUrl} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import {cn, formatBytes, isAudioMime, isPlayableMedia} from '@/lib/utils'
import {MediaPlayer} from './MediaPlayer'
import {useCopyPicture, useTrashMutations} from '@/hooks/usePictureEdit'
import {useExifDraft} from '@/hooks/useExifDraft'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {bestLoaded, recordImage, useImageCache} from '@/stores/imageCache'
import {carouselVisible, topBarVisible, useLightboxStore} from '@/stores/lightbox'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {Button} from '@/components/ui/button'
import {FileTypeIcon} from './FileTypeIcon'
import {OrientedContainImage} from './OrientedImage'
import {ZoomableArea} from './ZoomableArea'
import {LightboxCarousel} from './LightboxCarousel'

// Keyboard shortcuts for the full-screen player (Vidstack `keyTarget="document"`, so they work
// without focusing the player). ArrowLeft/Right are deliberately left unbound — the Lightbox uses
// them to move between pictures; j/l seek instead.
const MEDIA_KEY_SHORTCUTS = {
    togglePaused: 'k Space',
    toggleMuted: 'm',
    toggleFullscreen: 'f',
    togglePictureInPicture: 'i',
    toggleCaptions: 'c',
    seekBackward: 'j J',
    seekForward: 'l L',
    volumeUp: 'ArrowUp',
    volumeDown: 'ArrowDown',
    speedUp: '. >',
    slowDown: ', <',
} as const

/** Still-image viewer: zoom/pan + rotate controls. Rotation auto-commits (see `useExifDraft`). */
function LightboxImageWithDraft({picture, url, variant, blurhash, placeholderSrc, showRotate}: {
    picture: PictureDetail
    url: string
    variant: PictureVariant
    blurhash?: string | null
    placeholderSrc?: string | null
    showRotate: boolean
}) {
    const exif = useExifDraft(picture)
    const draftOrientation = exif.draft.orientation ? Number(exif.draft.orientation) : 1

    return (
        <>
            <ZoomableArea resetKey={picture.id}>
                <OrientedContainImage
                    src={url}
                    blurhash={blurhash}
                    placeholderSrc={placeholderSrc}
                    alt={picture.filename ?? ''}
                    orientation={draftOrientation}
                    width={picture.width}
                    height={picture.height}
                    onLoad={() => recordImage(picture.id, variant, url, true)}
                    onClick={(e) => e.stopPropagation()}
                />
            </ZoomableArea>
            <div
                className={cn(
                    'absolute bottom-4 left-1/2 z-10 flex -translate-x-1/2 items-center gap-2 transition-opacity',
                    showRotate ? 'opacity-100' : 'pointer-events-none opacity-0',
                )}
                onClick={(e) => e.stopPropagation()}
            >
                <button
                    onClick={() => exif.rotate('ccw')}
                    title="Rotate left"
                    aria-label="Rotate left"
                    className="flex h-9 w-9 items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70"
                >
                    <RotateCcw className="h-5 w-5"/>
                </button>
                <button
                    onClick={() => exif.rotate('cw')}
                    title="Rotate right"
                    aria-label="Rotate right"
                    className="flex h-9 w-9 items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70"
                >
                    <RotateCw className="h-5 w-5"/>
                </button>
            </div>
        </>
    )
}

/**
 * Video player fitted into the viewer exactly like an image: the box matches the video's aspect
 * ratio (when its dimensions are known) and grows to the largest size that fits the available area
 * (contain). Falls back to a plain full-width player when dimensions are unknown.
 */
function LightboxVideo({src, mime, title, width, height}: {
    src: string
    mime: string | null
    title?: string | null
    width: number | null
    height: number | null
}) {
    const ar = width && height ? width / height : null
    const ref = useRef<HTMLDivElement>(null)
    const [avail, setAvail] = useState({w: 0, h: 0})
    useEffect(() => {
        const el = ref.current
        if (!el) return
        setAvail({w: el.clientWidth, h: el.clientHeight})
        const ro = new ResizeObserver((entries) => {
            const r = entries[0].contentRect
            setAvail({w: r.width, h: r.height})
        })
        ro.observe(el)
        return () => ro.disconnect()
    }, [])

    // Largest box of the video's aspect ratio that fits the available area (contain).
    let box: { width: number; height: number } | null = null
    if (ar && avail.w > 0 && avail.h > 0) {
        let w = avail.w
        let h = w / ar
        if (h > avail.h) {
            h = avail.h
            w = h * ar
        }
        box = {width: w, height: h}
    }

    return (
        <div ref={ref} className="absolute inset-0 flex items-center justify-center" onClick={(e) => e.stopPropagation()}>
            {box ? (
                <div style={{width: box.width, height: box.height}}>
                    <MediaPlayer
                        src={src}
                        mime={mime}
                        title={title}
                        autoPlay
                        aspectRatio={`${width}/${height}`}
                        keyTarget="document"
                        keyShortcuts={MEDIA_KEY_SHORTCUTS}
                        className="h-full w-full"
                    />
                </div>
            ) : (
                // Dimensions unknown — let Vidstack size to the media once loaded.
                <MediaPlayer
                    src={src}
                    mime={mime}
                    title={title}
                    autoPlay
                    keyTarget="document"
                    keyShortcuts={MEDIA_KEY_SHORTCUTS}
                    className="max-h-full w-full max-w-5xl"
                />
            )}
        </div>
    )
}

/** Resolves the picture detail (for rotate) and renders the fitted image. */
function LightboxImage({item, url, variant, loading, showRotate}: {
    item: PictureListItem
    url: string | null
    variant: PictureVariant
    loading: boolean
    showRotate: boolean
}) {
    const {data: detail} = useQuery({
        queryKey: queryKeys.picture(item.id),
        queryFn: () => getPicture(item.id),
    })
    const mime = detail?.mime_type
    const media = isPlayableMedia(mime)

    // A lower-res variant already loaded by the browser (from the grid/carousel), shown behind the
    // large/original while it downloads so the viewer never flashes empty.
    const entry = useImageCache((s) => s.entries[item.id])
    const placeholderSrc = useMemo(() => bestLoaded(entry, 'large')?.url ?? null, [entry])

    // Video/audio: play the original inline (autoplay). Its presigned URL is fetched separately —
    // the `large` thumbnail variant (`url` above) is null for a non-thumbnailable media file.
    const {data: mediaUrl} = useQuery({
        queryKey: ['pictures', 'url', item.id, 'original'],
        queryFn: () => getPictureUrl(item.id, 'original'),
        enabled: media,
        staleTime: 10 * 60 * 1000,
    })

    if (media) {
        if (!mediaUrl?.url) {
            return <Loader2 className="h-8 w-8 animate-spin text-white/70" onClick={(e) => e.stopPropagation()}/>
        }
        // Audio: a centred player bar. Video: fills the viewer like an image, keeping its aspect ratio.
        if (isAudioMime(mime)) {
            return (
                <MediaPlayer
                    src={mediaUrl.url}
                    mime={mime ?? null}
                    title={item.filename}
                    autoPlay
                    keyTarget="document"
                    keyShortcuts={MEDIA_KEY_SHORTCUTS}
                    className="w-full max-w-2xl"
                />
            )
        }
        return (
            <LightboxVideo
                src={mediaUrl.url}
                mime={mime ?? null}
                title={item.filename}
                width={item.width}
                height={item.height}
            />
        )
    }

    // Non-image, non-media file (e.g. a PDF): show the file icon + hint, never in an `<img>` — even
    // when original-quality gives a non-null `original` URL. The header's Download still works.
    const nonImage = !!detail && detail.mime_type != null && !detail.mime_type.startsWith('image/')
    // Resolved with no viewable image (a non-thumbnailable image still pending, or the above) —
    // file icon + hint. While the detail is still loading show a spinner instead of the icon flash.
    if (nonImage || (!loading && !url)) {
        if (!detail) {
            return <Loader2 className="h-8 w-8 animate-spin text-white/70" onClick={(e) => e.stopPropagation()}/>
        }
        return (
            <div className="flex flex-col items-center justify-center gap-3 text-white/70" onClick={(e) => e.stopPropagation()}>
                <FileTypeIcon mime={detail?.mime_type} filename={item.filename} className="h-20 w-20"/>
                <p className="text-sm">No preview available. Use Download to get the original.</p>
            </div>
        )
    }
    // Once detail is loaded, render with rotate controls. Otherwise (URL still resolving, or detail
    // not loaded yet) render the image box with the blurhash / reused-thumbnail placeholder.
    if (detail && url) {
        return <LightboxImageWithDraft picture={detail} url={url} variant={variant} blurhash={item.blurhash} placeholderSrc={placeholderSrc}
                                       showRotate={showRotate}/>
    }
    return (
        <OrientedContainImage
            src={url ?? undefined}
            blurhash={item.blurhash}
            placeholderSrc={placeholderSrc}
            alt={item.filename ?? ''}
            orientation={item.orientation}
            width={item.width}
            height={item.height}
            onLoad={() => url && recordImage(item.id, variant, url, true)}
            onClick={(e) => e.stopPropagation()}
        />
    )
}

/** Shared style for a square icon button in the lightbox top bar. */
const ICON_BTN = 'flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-white/70 transition-colors hover:bg-white/10 hover:text-white disabled:opacity-50'

/** Compact icon toggle used by the top-bar controls; highlighted when `active`. */
function ToggleButton({active, onClick, title, children}: {
    active?: boolean
    onClick: () => void
    title: string
    children: ReactNode
}) {
    return (
        <button
            onClick={onClick}
            aria-label={title}
            aria-pressed={active}
            title={title}
            className={cn(ICON_BTN, active && 'bg-white/15 text-white hover:bg-white/20')}
        >
            {children}
        </button>
    )
}

/**
 * Full-screen carousel viewer. Driven by the `view=<pictureId>` URL param so it
 * is shareable and the back button closes it. Navigates across the items
 * currently loaded in the grid. Clicking the backdrop (outside the image) closes it.
 */
export function Lightbox({items, gridVariant = 'medium', loadMore}: {
    items: PictureListItem[]
    gridVariant?: PictureVariant
    /** Fetch the next grid page — called as the viewer nears the end of the loaded items. */
    loadMore?: () => void
}) {
    const [sp, setSp] = useSearchParams()
    const viewId = sp.get('view')
    const index = viewId ? items.findIndex((i) => i.id === viewId) : -1
    const open = index !== -1
    const current = open ? items[index] : null

    const containerRef = useRef<HTMLDivElement>(null)

    const setView = useCallback(
        (id: string | null) => {
            setSp(
                (prev) => {
                    const next = new URLSearchParams(prev)
                    if (id) next.set('view', id)
                    else next.delete('view')
                    return next
                },
                {replace: true},
            )
        },
        [setSp],
    )

    const select = useSelectionStore((s) => s.select)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const isMobile = useIsMobile()

    // Lightbox chrome state (§ feature request): top-bar / carousel visibility (per fullscreen mode),
    // original-quality (session), and fullscreen tracking.
    const barVisible = useLightboxStore(topBarVisible)
    const carVisible = useLightboxStore(carouselVisible)
    const fullscreen = useLightboxStore((s) => s.fullscreen)
    const originalQuality = useLightboxStore((s) => s.originalQuality)
    const toggleTopBar = useLightboxStore((s) => s.toggleTopBar)
    const toggleCarousel = useLightboxStore((s) => s.toggleCarousel)
    const toggleOriginalQuality = useLightboxStore((s) => s.toggleOriginalQuality)
    const setFullscreen = useLightboxStore((s) => s.setFullscreen)

    // When the chrome is hidden (top bar off), each control re-appears while the mouse is near its
    // edge: top → top bar, left/right → nav arrows, bottom → rotate buttons.
    const [edge, setEdge] = useState({top: false, left: false, right: false, bottom: false})
    const onMouseMove = useCallback((e: { clientX: number; clientY: number }) => {
        if (barVisible) return // chrome pinned — nothing to reveal
        const w = window.innerWidth
        const h = window.innerHeight
        const next = {top: e.clientY < 72, left: e.clientX < 96, right: e.clientX > w - 96, bottom: e.clientY > h - 140}
        setEdge((prev) =>
            prev.top === next.top && prev.left === next.left && prev.right === next.right && prev.bottom === next.bottom ? prev : next,
        )
    }, [barVisible])
    const showBar = barVisible || edge.top
    const showLeft = barVisible || edge.left
    const showRight = barVisible || edge.right
    const showRotate = barVisible || edge.bottom

    // Keep the store's fullscreen flag in sync with the browser.
    useEffect(() => {
        const onChange = () => setFullscreen(!!document.fullscreenElement)
        document.addEventListener('fullscreenchange', onChange)
        return () => document.removeEventListener('fullscreenchange', onChange)
    }, [setFullscreen])

    const toggleFullscreen = useCallback(() => {
        if (document.fullscreenElement) void document.exitFullscreen()
        else void containerRef.current?.requestFullscreen().catch(() => {
        })
    }, [])

    // Closing returns to the picture in context: select it so the sidebar shows its specs
    // (and on mobile open that drawer), which is more useful than landing back on the bare grid.
    const close = useCallback(() => {
        if (document.fullscreenElement) void document.exitFullscreen()
        if (viewId) {
            select(viewId)
            if (isMobile) openMobileDrawer('right')
        }
        setView(null)
    }, [viewId, select, isMobile, openMobileDrawer, setView])
    const goPrev = useCallback(() => {
        if (index > 0) setView(items[index - 1].id)
    }, [index, items, setView])
    const goNext = useCallback(() => {
        if (index >= 0 && index < items.length - 1) setView(items[index + 1].id)
    }, [index, items, setView])

    const {trash, restore} = useTrashMutations()
    const copy = useCopyPicture()

    // Trashing removes the picture from the underlying list — jump to the next picture first (or the
    // previous one if it was last) so the viewer stays open instead of collapsing to -1 and closing.
    const trashAndAdvance = useCallback(() => {
        if (!current) return
        const id = current.id
        const nextId = index < items.length - 1 ? items[index + 1].id : index > 0 ? items[index - 1].id : null
        if (nextId) setView(nextId)
        else close()
        trash.mutate(id)
    }, [current, index, items, setView, close, trash])

    useEffect(() => {
        if (!open || !current) return
        const onKey = (e: KeyboardEvent) => {
            // An open overlay (e.g. the trash confirm dialog) handles Escape/Enter first and
            // prevents default — don't also act on the lightbox in that case.
            if (e.defaultPrevented) return
            const t = e.target as HTMLElement | null
            if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
            if (e.key === 'Escape') {
                // In fullscreen the browser consumes Escape to exit; only close otherwise.
                if (!document.fullscreenElement) close()
            } else if (e.key === 'ArrowLeft') goPrev()
            else if (e.key === 'ArrowRight') goNext()
                // Delete / ⌘+Backspace trashes the picture currently in view, no confirmation —
            // a deliberate keyboard shortcut doesn't need the mouse-click confirm gate.
            else if (!current.deleted_at && (e.key === 'Delete' || (e.metaKey && e.key === 'Backspace'))) {
                e.preventDefault()
                trashAndAdvance()
            }
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [open, current, close, goPrev, goNext, trashAndAdvance])

    // Page in more items as the viewer approaches the end of what the grid has loaded.
    useEffect(() => {
        if (open && index >= 0 && index >= items.length - 5) loadMore?.()
    }, [open, index, items.length, loadMore])

    // The main image variant follows the original-quality toggle: `original` for the full blob,
    // otherwise the `large` thumbnail. `urlData.url` is null for non-thumbnailable formats.
    const variant: PictureVariant = originalQuality ? 'original' : 'large'
    const {data: urlData} = useQuery({
        queryKey: ['pictures', 'url', current?.id, variant],
        queryFn: () => getPictureUrl(current!.id, variant),
        enabled: !!current,
        staleTime: 10 * 60 * 1000,
    })

    // File size lives on the detail (not the list item); reuse the cached detail query.
    const {data: detail} = useQuery({
        queryKey: queryKeys.picture(current?.id ?? ''),
        queryFn: () => getPicture(current!.id),
        enabled: !!current,
    })

    const [downloading, setDownloading] = useState(false)
    const download = async () => {
        if (!current) return
        setDownloading(true)
        try {
            await downloadOriginal(current.id, current.filename)
        } catch (e) {
            toast.error('Could not download', {description: apiErrorMessage(e)})
        } finally {
            setDownloading(false)
        }
    }

    if (!open || !current) return null

    const trashed = !!current.deleted_at
    const ownerHandle = current.owner_username
        ? `@${current.owner_username}${current.owner_instance ? `:${current.owner_instance}` : ''}`
        : null
    const meta = [formatBytes(detail?.file_size), current.mime_type].filter(Boolean).join(' · ')

    // Portal to <body> so the viewer paints above the mobile sidebar drawer (rendered inside the
    // app tree); the trash confirm dialog still stacks on top as a later body portal.
    return createPortal(
        <div ref={containerRef} className="fixed inset-0 z-50 flex flex-col bg-black/95" onClick={close} onMouseMove={onMouseMove}>
            {/* Top bar. Pinned → in flow (the image sits below it, never under it); hidden → an
                overlay that slides in on a top-edge hover so revealing it doesn't shift the image. */}
            <div
                onClick={(e) => e.stopPropagation()}
                className={cn(
                    'z-40 flex items-center gap-1 px-3 py-1.5 text-white/90',
                    barVisible
                        ? 'shrink-0 bg-black/70'
                        : cn(
                            'absolute inset-x-0 top-0 bg-gradient-to-b from-black/80 to-transparent pb-6 transition-transform duration-200',
                            showBar ? 'translate-y-0' : '-translate-y-full',
                        ),
                )}
            >
                <div className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-sm leading-tight">{current.filename ?? 'Untitled'}</span>
                    <span className="flex flex-wrap items-center gap-x-2 text-xs leading-tight text-white/60">
                        <span>{index + 1} / {items.length}</span>
                        {meta && <span>· {meta}</span>}
                        {ownerHandle && <span className="text-white/70">· {ownerHandle}</span>}
                        {trashed && (
                            <span className="rounded bg-destructive/80 px-1 text-[10px] font-medium text-white">In trash</span>
                        )}
                        {!current.owned && current.owner_deleted_at && (
                            <span className="rounded bg-destructive/80 px-1 text-[10px] font-medium text-white">Owner deleting</span>
                        )}
                    </span>
                </div>

                {/* View controls: original quality, carousel, top-bar pin, fullscreen. */}
                <ToggleButton active={originalQuality} onClick={toggleOriginalQuality}
                              title={originalQuality ? 'Original quality: on' : 'Original quality: off'}>
                    <Sparkles className="h-4 w-4"/>
                </ToggleButton>
                <ToggleButton active={carVisible} onClick={toggleCarousel} title={carVisible ? 'Hide carousel' : 'Show carousel'}>
                    <Images className="h-4 w-4"/>
                </ToggleButton>
                <ToggleButton active={barVisible} onClick={toggleTopBar} title={barVisible ? 'Hide top bar' : 'Pin top bar'}>
                    <PanelTop className="h-4 w-4"/>
                </ToggleButton>
                <ToggleButton active={fullscreen} onClick={toggleFullscreen} title={fullscreen ? 'Exit full screen' : 'Full screen'}>
                    {fullscreen ? <Minimize2 className="h-4 w-4"/> : <Maximize2 className="h-4 w-4"/>}
                </ToggleButton>

                <div className="mx-1 h-5 w-px bg-white/15"/>

                <button
                    onClick={download}
                    disabled={downloading}
                    aria-label="Download original"
                    title="Download original"
                    className={ICON_BTN}
                >
                    {downloading ? <Loader2 className="h-4 w-4 animate-spin"/> : <Download className="h-4 w-4"/>}
                </button>
                {/* Copy ("rescue") a received picture into your own library (feature 11). */}
                {!current.owned && (
                    <button
                        onClick={() => copy.mutate(current.id)}
                        disabled={copy.isPending}
                        aria-label="Copy to my library"
                        title="Copy to my library"
                        className={ICON_BTN}
                    >
                        {copy.isPending ? <Loader2 className="h-4 w-4 animate-spin"/> : <Copy className="h-4 w-4"/>}
                    </button>
                )}
                {trashed ? (
                    <button
                        onClick={() => restore.mutate(current.id)}
                        disabled={restore.isPending}
                        aria-label="Restore"
                        title="Restore"
                        className="flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-sm text-white/80 transition-colors hover:bg-white/10 hover:text-white"
                    >
                        <ArchiveRestore className="h-4 w-4"/>
                        <span className="hidden sm:inline">Restore</span>
                    </button>
                ) : (
                    <ConfirmDialog
                        trigger={
                            <Button
                                variant="ghost"
                                size="icon"
                                aria-label="Move to trash"
                                title="Move to trash"
                                className={cn(ICON_BTN, 'hover:text-white')}
                            >
                                <Trash2 className="h-4 w-4"/>
                            </Button>
                        }
                        title="Move to trash?"
                        description={
                            current.owned
                                ? 'This photo will be hidden and permanently deleted after your retention window. Shared recipients see a deletion warning until then.'
                                : 'This removes the photo from your library locally. The owner\'s copy is unaffected.'
                        }
                        confirmLabel="Move to trash"
                        destructive
                        onConfirm={trashAndAdvance}
                    />
                )}
                <button onClick={close} aria-label="Close" className={ICON_BTN}>
                    <X className="h-4 w-4"/>
                </button>
            </div>

            {/* Image area (fills; the top bar overlays it, the carousel sits below it). */}
            <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-4">
                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        goPrev()
                    }}
                    disabled={index <= 0}
                    aria-label="Previous"
                    className={cn(
                        'absolute left-2 z-20 rounded-full bg-black/40 p-2 text-white transition-opacity hover:bg-black/60 disabled:opacity-25',
                        showLeft ? 'opacity-100' : 'pointer-events-none opacity-0',
                    )}
                >
                    <ChevronLeft className="h-6 w-6"/>
                </button>

                <LightboxImage item={current} url={urlData?.url ?? null} variant={variant} loading={urlData === undefined} showRotate={showRotate}/>

                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        goNext()
                    }}
                    disabled={index >= items.length - 1}
                    aria-label="Next"
                    className={cn(
                        'absolute right-2 z-20 rounded-full bg-black/40 p-2 text-white transition-opacity hover:bg-black/60 disabled:opacity-25',
                        showRight ? 'opacity-100' : 'pointer-events-none opacity-0',
                    )}
                >
                    <ChevronRight className="h-6 w-6"/>
                </button>
            </div>

            {/* Carousel below the image (in flow, so it never covers it). */}
            {carVisible && (
                <div className="shrink-0 bg-black/40" onClick={(e) => e.stopPropagation()}>
                    <LightboxCarousel items={items} currentId={current.id} gridVariant={gridVariant} onSelect={setView}/>
                </div>
            )}
        </div>,
        document.body,
    )
}
