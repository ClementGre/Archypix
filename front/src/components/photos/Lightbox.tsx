import {useCallback, useEffect} from 'react'
import {useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {ArchiveRestore, ChevronLeft, ChevronRight, Loader2, Trash2, X} from 'lucide-react'
import type {PictureListItem} from '@/lib/types'
import {getPictureUrl} from '@/api/pictures'
import {useTrashMutations} from '@/hooks/usePictureEdit'
import {OrientedContainImage} from './OrientedImage'

/**
 * Full-screen carousel viewer. Driven by the `view=<pictureId>` URL param so it
 * is shareable and the back button closes it. Navigates across the items
 * currently loaded in the grid.
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

    const close = useCallback(() => setView(null), [setView])
    const goPrev = useCallback(() => {
        if (index > 0) setView(items[index - 1].id)
    }, [index, items, setView])
    const goNext = useCallback(() => {
        if (index >= 0 && index < items.length - 1) setView(items[index + 1].id)
    }, [index, items, setView])

    useEffect(() => {
        if (!open) return
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') close()
            else if (e.key === 'ArrowLeft') goPrev()
            else if (e.key === 'ArrowRight') goNext()
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [open, close, goPrev, goNext])

    const {data: url, isFetching} = useQuery({
        queryKey: ['pictures', 'url', current?.id, 'large'],
        queryFn: () => getPictureUrl(current!.id, 'large'),
        enabled: !!current,
        staleTime: 10 * 60 * 1000,
    })

    const {trash, restore} = useTrashMutations()

    if (!open || !current) return null

    const trashed = !!current.deleted_at

    return (
        <div className="fixed inset-0 z-50 flex flex-col bg-black/90" onClick={close}>
            <div
                className="flex items-center gap-3 px-4 py-3 text-white/90"
                onClick={(e) => e.stopPropagation()}
            >
                <span className="min-w-0 flex-1 truncate text-sm">{current.filename ?? 'Untitled'}</span>
                <span className="text-xs text-white/60">
          {index + 1} / {items.length}
        </span>
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
                    <button
                        onClick={() => trash.mutate(current.id)}
                        disabled={trash.isPending}
                        aria-label="Move to trash"
                        title="Move to trash"
                        className="rounded p-1 hover:bg-white/10"
                    >
                        <Trash2 className="h-5 w-5"/>
                    </button>
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

                <div className="relative h-full w-full" onClick={(e) => e.stopPropagation()}>
                    {url ? (
                        <OrientedContainImage
                            src={url.url}
                            alt={current.filename ?? ''}
                            orientation={current.orientation}
                            width={current.width}
                            height={current.height}
                        />
                    ) : (
                        isFetching && (
                            <div className="flex h-full w-full items-center justify-center">
                                <Loader2 className="h-8 w-8 animate-spin text-white/70"/>
                            </div>
                        )
                    )}
                </div>

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
        </div>
    )
}
