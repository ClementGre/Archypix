import {useCallback, useEffect, useRef, useState} from 'react'
import {createPortal} from 'react-dom'
import {useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {toast} from 'sonner'
import {ArchiveRestore, ChevronLeft, ChevronRight, Copy, Download, Loader2, RotateCcw, RotateCw, Trash2, X} from 'lucide-react'
import type {PictureDetail, PictureListItem} from '@/lib/types'
import {downloadOriginal, getPicture, getPictureUrl} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import {isAudioMime, isPlayableMedia} from '@/lib/utils'
import {MediaPlayer} from './MediaPlayer'
import {useCopyPicture, useTrashMutations} from '@/hooks/usePictureEdit'
import {useExifDraft} from '@/hooks/useExifDraft'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {Button} from '@/components/ui/button'
import {FileTypeIcon} from './FileTypeIcon'
import {OrientedContainImage} from './OrientedImage'

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

/** Image + rotate controls for the current picture. Rotation auto-commits (see `useExifDraft`). */
function LightboxImageWithDraft({picture, url, blurhash}: { picture: PictureDetail; url: string; blurhash?: string | null }) {
    const exif = useExifDraft(picture)
    const draftOrientation = exif.draft.orientation ? Number(exif.draft.orientation) : 1

    return (
        <>
            <OrientedContainImage
                src={url}
                blurhash={blurhash}
                alt={picture.filename ?? ''}
                orientation={draftOrientation}
                width={picture.width}
                height={picture.height}
                onClick={(e) => e.stopPropagation()}
            />
            <div
                className="absolute bottom-4 left-1/2 z-10 flex -translate-x-1/2 items-center gap-2"
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
function LightboxImage({item, url, loading}: { item: PictureListItem; url: string | null; loading: boolean }) {
    const {data: detail} = useQuery({
        queryKey: queryKeys.picture(item.id),
        queryFn: () => getPicture(item.id),
    })
    const mime = detail?.mime_type
    const media = isPlayableMedia(mime)

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

    // Resolved with no viewable image (a non-thumbnailable format like a PDF) — file icon + hint;
    // the header's Download button still works on the original. While the detail is still loading
    // (so we can't yet tell it's a playable media file), show a spinner instead of the icon flash.
    if (!loading && !url) {
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
    // not loaded yet) render the image box with the blurhash placeholder — which shows immediately,
    // before the large image (and even its URL) arrives, then fades out on load.
    if (detail && url) {
        return <LightboxImageWithDraft picture={detail} url={url} blurhash={item.blurhash}/>
    }
    return (
        <OrientedContainImage
            src={url ?? undefined}
            blurhash={item.blurhash}
            alt={item.filename ?? ''}
            orientation={item.orientation}
            width={item.width}
            height={item.height}
            onClick={(e) => e.stopPropagation()}
        />
    )
}

/**
 * Full-screen carousel viewer. Driven by the `view=<pictureId>` URL param so it
 * is shareable and the back button closes it. Navigates across the items
 * currently loaded in the grid. Clicking the backdrop (outside the image) closes it.
 */
export function Lightbox({items}: { items: PictureListItem[] }) {
    const [sp, setSp] = useSearchParams()
    const viewId = sp.get('view')
    const index = viewId ? items.findIndex((i) => i.id === viewId) : -1
    const open = index !== -1
    const current = open ? items[index] : null

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

    // Closing returns to the picture in context: select it so the sidebar shows its specs
    // (and on mobile open that drawer), which is more useful than landing back on the bare grid.
    const close = useCallback(() => {
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

    // Trashing removes the picture from the underlying list (async, on refetch) — jump to the
    // next picture first (or the previous one if it was last) so the viewer stays open instead
    // of collapsing to -1 and auto-closing; only close if it was the last picture left.
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
            if (e.key === 'Escape') close()
            else if (e.key === 'ArrowLeft') goPrev()
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

    // Full screen always requests the `large` variant. `urlData.url` is null for non-thumbnailable
    // formats; `urlData === undefined` means the request is still in flight.
    const {data: urlData} = useQuery({
        queryKey: ['pictures', 'url', current?.id, 'large'],
        queryFn: () => getPictureUrl(current!.id, 'large'),
        enabled: !!current,
        staleTime: 10 * 60 * 1000,
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

    // Portal to <body> so the viewer paints above the mobile sidebar drawer (rendered inside the
    // app tree); the trash confirm dialog still stacks on top as a later body portal.
    return createPortal(
        <div className="fixed inset-0 z-50 flex flex-col bg-black/90" onClick={close}>
            <div
                className="flex items-center gap-3 px-4 py-3 text-white/90"
                onClick={(e) => e.stopPropagation()}
            >
                <span className="min-w-0 flex-1 truncate text-sm">{current.filename ?? 'Untitled'}</span>
                <span className="text-xs text-white/60">
          {index + 1} / {items.length}
        </span>
                <button
                    onClick={download}
                    disabled={downloading}
                    aria-label="Download original"
                    title="Download original"
                    className="rounded p-1 hover:bg-white/10 disabled:opacity-50"
                >
                    {downloading ? <Loader2 className="h-5 w-5 animate-spin"/> : <Download className="h-5 w-5"/>}
                </button>
                {/* Copy ("rescue") a received picture into your own library (feature 11). */}
                {!current.owned && (
                    <button
                        onClick={() => copy.mutate(current.id)}
                        disabled={copy.isPending}
                        aria-label="Copy to my library"
                        title="Copy to my library"
                        className="rounded p-1 hover:bg-white/10 disabled:opacity-50"
                    >
                        {copy.isPending ? <Loader2 className="h-5 w-5 animate-spin"/> : <Copy className="h-5 w-5"/>}
                    </button>
                )}
                {trashed ? (
                    <button
                        onClick={() => restore.mutate(current.id)}
                        disabled={restore.isPending}
                        aria-label="Restore"
                        title="Restore"
                        className="flex items-center gap-1.5 rounded px-2 py-1 text-sm hover:bg-white/10"
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
                                className="h-auto w-auto rounded p-1 text-white/90 hover:bg-white/10 hover:text-white"
                            >
                                <Trash2 className="h-5 w-5"/>
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
                <button onClick={close} aria-label="Close" className="rounded p-1 hover:bg-white/10">
                    <X className="h-5 w-5"/>
                </button>
            </div>

            <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-4">
                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        goPrev()
                    }}
                    disabled={index <= 0}
                    aria-label="Previous"
                    className="absolute left-2 z-10 rounded-full bg-black/40 p-2 text-white hover:bg-black/60 disabled:opacity-25"
                >
                    <ChevronLeft className="h-6 w-6"/>
                </button>

                <LightboxImage item={current} url={urlData?.url ?? null} loading={urlData === undefined}/>

                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        goNext()
                    }}
                    disabled={index >= items.length - 1}
                    aria-label="Next"
                    className="absolute right-2 z-10 rounded-full bg-black/40 p-2 text-white hover:bg-black/60 disabled:opacity-25"
                >
                    <ChevronRight className="h-6 w-6"/>
                </button>
            </div>
        </div>,
        document.body,
    )
}
