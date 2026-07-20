import {useCallback, useEffect, useRef, useState} from 'react'
import {useQuery} from '@tanstack/react-query'
import {ChevronLeft, ChevronRight, Download, Loader2, X} from 'lucide-react'
import {getPublicPictureUrl} from '@/api/publicShares'
import type {PictureListItem} from '@/lib/types'
import {isPlayableMedia} from '@/lib/utils'
import {OrientedContainImage} from '@/components/photos/OrientedImage'
import {FileTypeIcon} from '@/components/photos/FileTypeIcon'
import {MediaPlayer} from '@/components/photos/MediaPlayer'
import {usePublicShare} from '@/components/public/context'
import {toast} from 'sonner'

export function PublicLightbox({
                                   items,
                                   startId,
                                   onClose,
                                   onLanded,
                               }: {
    items: PictureListItem[]
    startId: string
    onClose: () => void
    onLanded: (id: string) => void
}) {
    const {backendUrl, token, session, meta} = usePublicShare()
    const [index, setIndex] = useState(() => Math.max(0, items.findIndex((i) => i.id === startId)))
    const item = items[index]
    // Track the currently-viewed id so the "select on close" cleanup lands on it (not the start item).
    const currentIdRef = useRef<string | undefined>(item?.id)
    currentIdRef.current = item?.id

    const go = useCallback(
        (delta: number) => {
            setIndex((i) => Math.min(items.length - 1, Math.max(0, i + delta)))
        },
        [items.length],
    )

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') onClose()
            else if (e.key === 'ArrowLeft') go(-1)
            else if (e.key === 'ArrowRight') go(1)
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [go, onClose])

    // Landing on close: select the currently-viewed picture so its specs show in the side panel.
    useEffect(() => {
        return () => {
            const id = currentIdRef.current
            if (id) onLanded(id)
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    const playable = isPlayableMedia(item?.mime_type)
    const variant = playable ? 'original' : 'large'
    const urlQ = useQuery({
        queryKey: ['publicUrl', backendUrl, token, item?.id, variant],
        queryFn: () => getPublicPictureUrl(backendUrl, token, item!.id, variant, session),
        enabled: !!item,
        retry: false,
    })

    if (!item) return null

    const download = async () => {
        try {
            const url = await getPublicPictureUrl(backendUrl, token, item.id, 'original', session)
            if (!url) throw new Error('unavailable')
            const res = await fetch(url)
            const blob = await res.blob()
            const a = document.createElement('a')
            a.href = URL.createObjectURL(blob)
            a.download = item.filename ?? item.id
            a.click()
            URL.revokeObjectURL(a.href)
        } catch {
            toast.error('Download is not available for this album.')
        }
    }

    return (
        <div className="fixed inset-0 z-50 flex flex-col bg-black/95" onClick={onClose}>
            <div
                className="flex items-center justify-between gap-2 px-4 py-2 text-sm text-white/90"
                onClick={(e) => e.stopPropagation()}
            >
                <div className="min-w-0 truncate">
                    <span className="font-medium">{item.filename ?? 'Untitled'}</span>
                    <span className="ml-2 text-white/50">
                        {index + 1} / {items.length}
                    </span>
                </div>
                <div className="flex items-center gap-1">
                    {meta.permissions.allow_originals && (
                        <IconBtn onClick={download} label="Download original">
                            <Download className="h-5 w-5"/>
                        </IconBtn>
                    )}
                    <IconBtn onClick={onClose} label="Close">
                        <X className="h-5 w-5"/>
                    </IconBtn>
                </div>
            </div>

            <div className="relative flex min-h-0 flex-1 items-center justify-center" onClick={(e) => e.stopPropagation()}>
                {index > 0 && (
                    <NavBtn side="left" onClick={() => go(-1)}>
                        <ChevronLeft className="h-8 w-8"/>
                    </NavBtn>
                )}
                <div className="flex h-full w-full items-center justify-center p-2 sm:p-8">
                    {urlQ.isPending ? (
                        <Loader2 className="h-8 w-8 animate-spin text-white/70"/>
                    ) : urlQ.data ? (
                        playable ? (
                            <MediaPlayer src={urlQ.data} mime={item.mime_type ?? ''} title={item.filename ?? ''} autoPlay/>
                        ) : (
                            <div className="relative h-full w-full">
                                <OrientedContainImage
                                    src={urlQ.data}
                                    alt={item.filename ?? 'photo'}
                                    orientation={item.orientation}
                                    blurhash={item.blurhash}
                                    width={item.width}
                                    height={item.height}
                                />
                            </div>
                        )
                    ) : (
                        <div className="flex flex-col items-center gap-2 text-white/70">
                            <FileTypeIcon mime={item.mime_type} filename={item.filename} className="h-16 w-16"/>
                            <p className="text-sm">No preview available.</p>
                        </div>
                    )}
                </div>
                {index < items.length - 1 && (
                    <NavBtn side="right" onClick={() => go(1)}>
                        <ChevronRight className="h-8 w-8"/>
                    </NavBtn>
                )}
            </div>
        </div>
    )
}

function IconBtn({children, onClick, label}: { children: React.ReactNode; onClick: () => void; label: string }) {
    return (
        <button
            onClick={onClick}
            aria-label={label}
            className="rounded-full p-2 text-white/80 transition hover:bg-white/10 hover:text-white"
        >
            {children}
        </button>
    )
}

function NavBtn({children, side, onClick}: { children: React.ReactNode; side: 'left' | 'right'; onClick: () => void }) {
    return (
        <button
            onClick={onClick}
            className={`absolute top-1/2 -translate-y-1/2 rounded-full p-2 text-white/70 transition hover:bg-white/10 hover:text-white ${
                side === 'left' ? 'left-2' : 'right-2'
            }`}
        >
            {children}
        </button>
    )
}
