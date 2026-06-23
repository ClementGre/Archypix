import {useEffect, useMemo, useRef} from 'react'
import {ArchiveRestore, ImageOff, Loader2, Trash2} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {usePictures} from '@/hooks/usePictures'
import {useTrashMutations} from '@/hooks/usePictureEdit'
import {useSettings} from '@/hooks/useSettings'
import {OrientedImage} from '@/components/photos/OrientedImage'
import {apiErrorMessage} from '@/api/client'
import {formatDateTime} from '@/lib/utils'
import {deadlineLabel, ownedPurgeAt} from '@/lib/trash'
import type {PictureListItem} from '@/lib/types'

function TrashCard({item, retentionDays}: { item: PictureListItem; retentionDays: number }) {
    const {restore} = useTrashMutations()
    const purgeLabel = item.owned
        ? deadlineLabel(ownedPurgeAt(item.deleted_at, retentionDays))
        : null

    return (
        <li className="group relative flex flex-col overflow-hidden rounded-md border border-border bg-card">
            <div className="relative aspect-square w-full overflow-hidden bg-checkerboard">
                {item.thumbnail_url ? (
                    <OrientedImage
                        src={item.thumbnail_url}
                        alt={item.filename ?? ''}
                        orientation={item.orientation}
                        width={item.width}
                        height={item.height}
                        className="opacity-90"
                    />
                ) : (
                    <div className="flex h-full items-center justify-center text-muted-foreground">
                        <ImageOff className="h-7 w-7"/>
                    </div>
                )}

                {!item.owned && item.owner_username && (
                    <span className="absolute bottom-1 left-1 rounded bg-black/55 px-1 text-[10px] leading-4 text-white">
                        @{item.owner_username}
                    </span>
                )}

                <Button
                    variant="secondary"
                    size="sm"
                    className="absolute right-1 top-1 h-7 gap-1.5 opacity-0 transition-opacity group-hover:opacity-100"
                    disabled={restore.isPending}
                    onClick={() => restore.mutate(item.id)}
                >
                    <ArchiveRestore className="h-3.5 w-3.5"/> Restore
                </Button>
            </div>

            <div className="flex flex-col gap-0.5 p-2">
                <p className="truncate text-xs font-medium" title={item.filename ?? undefined}>
                    {item.filename ?? 'Untitled'}
                </p>
                <p className="text-[11px] text-muted-foreground">Trashed {formatDateTime(item.deleted_at)}</p>
                {purgeLabel ? (
                    <p className="text-[11px] text-destructive">Deleted {purgeLabel}</p>
                ) : (
                    <p className="text-[11px] text-muted-foreground">Hidden locally · owner's copy untouched</p>
                )}
            </div>
        </li>
    )
}

export default function TrashPage() {
    const {data: settings} = useSettings()
    const retentionDays = settings?.trash_retention_days ?? 30

    const {data, isPending, isError, error, fetchNextPage, hasNextPage, isFetchingNextPage} = usePictures({
        scope: 'all',
        includeDeleted: true,
    })

    // The list endpoint has no "trashed only" filter — keep just the soft-deleted rows.
    const items = useMemo(
        () => (data?.pages.flatMap((p) => p.items) ?? []).filter((it) => it.deleted_at),
        [data],
    )

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

    return (
        <div className="h-full overflow-y-auto">
            <div className="mx-auto max-w-6xl p-6">
                <div className="mb-1 flex items-center gap-2">
                    <Trash2 className="h-5 w-5 text-muted-foreground"/>
                    <h1 className="text-xl font-semibold">Trash</h1>
                </div>
                <p className="mb-6 text-sm text-muted-foreground">
                    Trashed photos can be restored. Your own photos are permanently deleted{' '}
                    <span className="font-medium">{retentionDays} days</span> after trashing; received
                    photos are only hidden locally (the owner's copy is untouched).
                </p>

                {isPending ? (
                    <ul className="grid list-none grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3 p-0">
                        {Array.from({length: 8}).map((_, i) => (
                            <li key={i} className="aspect-square animate-pulse rounded-md bg-muted"/>
                        ))}
                    </ul>
                ) : isError ? (
                    <div className="flex flex-col items-center gap-2 py-16 text-center text-sm text-muted-foreground">
                        <p>Could not load the trash.</p>
                        <p className="text-xs">{apiErrorMessage(error)}</p>
                    </div>
                ) : items.length === 0 ? (
                    <div className="flex flex-col items-center gap-2 py-16 text-center text-sm text-muted-foreground">
                        <Trash2 className="h-8 w-8"/>
                        <p>The trash is empty.</p>
                    </div>
                ) : (
                    <ul className="grid list-none grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3 p-0">
                        {items.map((it) => (
                            <TrashCard key={it.id} item={it} retentionDays={retentionDays}/>
                        ))}
                    </ul>
                )}

                <div ref={sentinel} className="flex h-12 items-center justify-center">
                    {isFetchingNextPage && <Loader2 className="h-5 w-5 animate-spin text-muted-foreground"/>}
                </div>
            </div>
        </div>
    )
}
