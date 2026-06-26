import {useState} from 'react'
import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {toast} from 'sonner'
import {Check, Copy, Loader2, Trash2} from 'lucide-react'
import {getPictureCopies, keepCopy, type PictureCopy} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {invalidatePicturesAndTags} from '@/lib/invalidation'
import {formatDateTime} from '@/lib/utils'
import {Section} from '@/components/photos/detail/Section'
import {Button} from '@/components/ui/button'

/** Human label + colour for a dedup state. */
function stateBadge(state: PictureCopy['state']): { label: string; cls: string } {
    switch (state) {
        case 'live':
            return {label: 'Shown', cls: 'bg-emerald-500/15 text-emerald-500'}
        case 'manual':
            return {label: 'In trash', cls: 'bg-muted text-muted-foreground'}
        case 'boomerang':
            return {label: 'Rejected', cls: 'bg-amber-500/15 text-amber-600'}
        case 'content_dedupe':
            return {label: 'Duplicate', cls: 'bg-muted text-muted-foreground'}
        default:
            return {label: 'Deleted', cls: 'bg-muted text-muted-foreground'}
    }
}

/** How `copy` differs from the reference survivor: same image / EXIF-only / different content. */
function diffLabel(copy: PictureCopy, ref: PictureCopy | undefined): string | null {
    if (!ref || copy.id === ref.id) return null
    if (copy.content_hash && ref.content_hash) {
        if (copy.content_hash === ref.content_hash) {
            return copy.file_hash === ref.file_hash ? 'identical' : 'same image · EXIF differs'
        }
        return 'different image'
    }
    // Fall back to file_hash when content_hash is unavailable (unstrippable format).
    if (copy.file_hash && ref.file_hash) {
        return copy.file_hash === ref.file_hash ? 'identical' : 'different image'
    }
    return null
}

function ownerLabel(copy: PictureCopy): string {
    if (copy.owned) {
        if (copy.copy_source_owner_username) {
            return `Your copy of @${copy.copy_source_owner_username}${copy.copy_source_owner_instance ? `:${copy.copy_source_owner_instance}` : ''}`
        }
        return 'You'
    }
    return `@${copy.owner_username ?? '?'}:${copy.owner_instance ?? '?'}`
}

/**
 * Foldable "Copies" section (feature 11 §5.5): lists every physical copy in this picture's
 * content-dedup group — the shown survivor plus hidden duplicates / trashed / rejected siblings —
 * with each copy's owner, last-edit date, how it differs (same image vs EXIF-only vs different
 * content), and a control to make a chosen copy the kept (shown) one. Lazily fetched on open.
 */
export function CopiesSection({pictureId}: { pictureId: string }) {
    const [open, setOpen] = useState(false)
    const queryClient = useQueryClient()

    const {data: copies, isLoading} = useQuery({
        queryKey: ['pictures', 'copies', pictureId],
        queryFn: () => getPictureCopies(pictureId),
        enabled: open,
        staleTime: 30_000,
    })

    const keep = useMutation({
        mutationFn: keepCopy,
        onSuccess: () => {
            invalidatePicturesAndTags(queryClient)
            queryClient.invalidateQueries({queryKey: ['pictures', 'copies']})
            toast.success('Kept this version')
        },
        onError: (e: unknown) => toast.error('Could not change version', {description: apiErrorMessage(e)}),
    })

    // The survivor (live) is the reference for the difference labels.
    const reference = copies?.find((c) => c.state === 'live') ?? copies?.[0]

    return (
        <Section
            id="copies"
            title="Copies"
            count={copies?.length}
            defaultOpen={false}
            open={open}
            onOpenChange={setOpen}
        >
            {isLoading ? (
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Loader2 className="h-3.5 w-3.5 animate-spin"/> Loading copies…
                </div>
            ) : !copies || copies.length <= 1 ? (
                <span className="text-xs text-muted-foreground">No other copies of this picture.</span>
            ) : (
                <div className="space-y-2">
                    {copies.map((c) => {
                        const badge = stateBadge(c.state)
                        const diff = diffLabel(c, reference)
                        const isShown = c.state === 'live'
                        return (
                            <div key={c.id} className="rounded-md border border-border px-2.5 py-1.5 text-xs">
                                <div className="flex items-center justify-between gap-2">
                                    <span className="flex items-center gap-1.5 truncate">
                                        {c.state === 'manual' || c.state === 'boomerang' ? (
                                            <Trash2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground"/>
                                        ) : (
                                            <Copy className="h-3.5 w-3.5 shrink-0 text-muted-foreground"/>
                                        )}
                                        <span className="truncate">{ownerLabel(c)}</span>
                                    </span>
                                    <span className={`shrink-0 rounded px-1.5 py-0.5 font-medium ${badge.cls}`}>
                                        {badge.label}
                                    </span>
                                </div>
                                <div className="mt-1 flex items-center justify-between gap-2 text-muted-foreground">
                                    <span>
                                        {formatDateTime(c.updated_at)}
                                        {diff && <span className="ml-1.5">· {diff}</span>}
                                    </span>
                                    {isShown ? (
                                        <span className="flex items-center gap-1 text-emerald-500">
                                            <Check className="h-3.5 w-3.5"/> Kept
                                        </span>
                                    ) : (
                                        <Button
                                            variant="ghost"
                                            size="sm"
                                            className="h-6 px-2 text-xs"
                                            disabled={keep.isPending}
                                            onClick={() => keep.mutate(c.id)}
                                        >
                                            Keep this
                                        </Button>
                                    )}
                                </div>
                            </div>
                        )
                    })}
                </div>
            )}
        </Section>
    )
}
