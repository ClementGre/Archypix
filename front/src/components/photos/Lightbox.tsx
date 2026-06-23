import {useCallback, useEffect, useState} from 'react'
import {createPortal} from 'react-dom'
import {useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {toast} from 'sonner'
import {ArchiveRestore, ChevronLeft, ChevronRight, Download, Loader2, RotateCcw, RotateCw, Trash2, X} from 'lucide-react'
import type {PictureDetail, PictureListItem} from '@/lib/types'
import {downloadOriginal, getPicture, getPictureUrl} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {queryKeys} from '@/lib/constants'
import {useTrashMutations} from '@/hooks/usePictureEdit'
import {useExifDraft} from '@/hooks/useExifDraft'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {Button} from '@/components/ui/button'
import {OrientedContainImage} from './OrientedImage'

/** Image + rotate controls for the current picture. Rotation auto-commits (see `useExifDraft`). */
function LightboxImageWithDraft({picture, url}: { picture: PictureDetail; url: string }) {
    const exif = useExifDraft(picture)
    const draftOrientation = exif.draft.orientation ? Number(exif.draft.orientation) : 1

    return (
        <>
            <OrientedContainImage
                src={url}
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

/** Resolves the picture detail (for rotate) and renders the fitted image. */
function LightboxImage({item, url}: { item: PictureListItem; url?: string }) {
    const {data: detail} = useQuery({
        queryKey: queryKeys.picture(item.id),
        queryFn: () => getPicture(item.id),
    })

    if (!url) {
        return (
            <div className="flex h-full w-full items-center justify-center">
                <Loader2 className="h-8 w-8 animate-spin text-white/70"/>
            </div>
        )
    }
    // Detail not loaded yet — show the image at its stored orientation; rotate appears once loaded.
    if (!detail) {
        return (
            <OrientedContainImage
                src={url}
                alt={item.filename ?? ''}
                orientation={item.orientation}
                width={item.width}
                height={item.height}
                onClick={(e) => e.stopPropagation()}
            />
        )
    }
    return <LightboxImageWithDraft picture={detail} url={url}/>
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

    useEffect(() => {
        if (!open) return
        const onKey = (e: KeyboardEvent) => {
            // An open overlay (e.g. the trash confirm dialog) handles Escape first and prevents
            // default — don't also close the lightbox in that case.
            if (e.defaultPrevented) return
            if (e.key === 'Escape') close()
            else if (e.key === 'ArrowLeft') goPrev()
            else if (e.key === 'ArrowRight') goNext()
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [open, close, goPrev, goNext])

    // Full screen always requests the `large` variant.
    const {data: url} = useQuery({
        queryKey: ['pictures', 'url', current?.id, 'large'],
        queryFn: () => getPictureUrl(current!.id, 'large'),
        enabled: !!current,
        staleTime: 10 * 60 * 1000,
    })

    const {trash, restore} = useTrashMutations()

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
                        onConfirm={() => trash.mutate(current.id)}
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

                <LightboxImage item={current} url={url?.url}/>

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
