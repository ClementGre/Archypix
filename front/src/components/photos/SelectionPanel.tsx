import {Link} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {toast} from 'sonner'
import {ExternalLink, ImageIcon, Tag as TagIcon, Trash2, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Separator} from '@/components/ui/separator'
import {getPicture, getPictureUrl} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import {useBatchEditTags, usePictureTags} from '@/hooks/useTags'
import {useSelectionStore} from '@/stores/selection'
import {TagPicker} from '@/components/tags/TagPicker'
import {queryKeys} from '@/lib/constants'
import {TagPath} from '@/lib/utils'

function formatBytes(bytes: number | null): string {
    if (!bytes) return '—'
    const units = ['B', 'KB', 'MB', 'GB']
    let n = bytes
    let i = 0
    while (n >= 1024 && i < units.length - 1) {
        n /= 1024
        i++
    }
    return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${units[i]}`
}

function formatDate(iso: string | null): string {
    if (!iso) return '—'
    const d = new Date(iso)
    return Number.isNaN(d.getTime()) ? '—' : d.toLocaleString()
}

function InfoRow({label, value}: { label: string; value: string }) {
    return (
        <div className="flex items-baseline justify-between gap-2 text-sm">
            <span className="text-muted-foreground">{label}</span>
            <span className="truncate text-right">{value}</span>
        </div>
    )
}

function TagEditor({pictureIds, currentTags}: { pictureIds: string[]; currentTags: string[] }) {
    const batch = useBatchEditTags()
    const add = (wire: string) =>
        batch.mutate(
            {picture_ids: pictureIds, add_tags: [wire]},
            {onError: (e) => toast.error('Could not add tag', {description: apiErrorMessage(e)})},
        )
    const remove = (wire: string) =>
        batch.mutate(
            {picture_ids: pictureIds, remove_tags: [wire]},
            {onError: (e) => toast.error('Could not remove tag', {description: apiErrorMessage(e)})},
        )

    return (
        <div className="space-y-2">
            <div className="flex flex-wrap gap-1.5">
                {currentTags.length > 0 ? (
                    currentTags.map((t) => (
                        <Badge key={t} variant="secondary" className="gap-1 font-normal">
                            {TagPath.toDisplay(t)}
                            <button onClick={() => remove(t)} aria-label={`Remove ${t}`} className="ml-0.5">
                                <X className="h-3 w-3"/>
                            </button>
                        </Badge>
                    ))
                ) : (
                    <span className="text-xs text-muted-foreground">No tags.</span>
                )}
            </div>
            <TagPicker onSelect={add} excludePaths={currentTags}/>
        </div>
    )
}

function SinglePicture({id}: { id: string }) {
    const {data: picture, isPending} = useQuery({
        queryKey: queryKeys.picture(id),
        queryFn: () => getPicture(id),
    })
    const {data: preview} = useQuery({
        queryKey: ['pictures', 'url', id, 'medium'],
        queryFn: () => getPictureUrl(id, 'medium'),
        staleTime: 10 * 60 * 1000,
    })
    const {data: tags} = usePictureTags(id)

    if (isPending || !picture) {
        return <div className="h-40 animate-pulse rounded-md bg-muted"/>
    }

    return (
        <div className="space-y-4">
            <div className="overflow-hidden rounded-md bg-muted">
                {preview ? (
                    <img src={preview.url} alt={picture.filename ?? ''} className="max-h-64 w-full object-contain"/>
                ) : (
                    <div className="flex h-40 items-center justify-center text-muted-foreground">
                        <ImageIcon className="h-8 w-8"/>
                    </div>
                )}
            </div>

            <div>
                <p className="truncate font-medium" title={picture.filename ?? undefined}>
                    {picture.filename ?? 'Untitled'}
                </p>
                <Button asChild variant="link" className="h-auto p-0 text-xs">
                    <Link to={`/photos/${picture.id}`}>
                        Open details <ExternalLink className="ml-1 h-3 w-3"/>
                    </Link>
                </Button>
            </div>

            <Separator/>

            <div className="space-y-1.5">
                <InfoRow label="Dimensions" value={picture.width && picture.height ? `${picture.width} × ${picture.height}` : '—'}/>
                <InfoRow label="Size" value={formatBytes(picture.file_size)}/>
                <InfoRow label="Taken" value={formatDate(picture.captured_at)}/>
                <InfoRow label="Added" value={formatDate(picture.ingested_at)}/>
                {picture.owner_username && (
                    <InfoRow label="Owner" value={`@${picture.owner_username}:${picture.owner_instance_domain ?? '?'}`}/>
                )}
                <InfoRow label="EXIF sync" value={picture.exif_sync_status}/>
            </div>

            <Separator/>

            <div>
                <div className="mb-2 flex items-center gap-1.5 text-sm font-medium">
                    <TagIcon className="h-3.5 w-3.5"/> Tags
                </div>
                <TagEditor pictureIds={[id]} currentTags={tags ?? []}/>
            </div>
        </div>
    )
}

function MultiSelection({ids, onClear}: { ids: string[]; onClear: () => void }) {
    return (
        <div className="space-y-4">
            <p className="text-sm">
                <span className="font-medium">{ids.length}</span> photos selected
            </p>
            <Separator/>
            <div>
                <div className="mb-2 flex items-center gap-1.5 text-sm font-medium">
                    <TagIcon className="h-3.5 w-3.5"/> Tag all selected
                </div>
                <TagEditor pictureIds={ids} currentTags={[]}/>
            </div>
            <Separator/>
            {/* Move-to-trash wires in the deletion milestone. */}
            <Button variant="outline" className="w-full justify-start gap-2 text-destructive" disabled>
                <Trash2 className="h-4 w-4"/> Move to trash
            </Button>
            <Button variant="ghost" className="w-full gap-2" onClick={onClear}>
                <X className="h-4 w-4"/> Clear selection
            </Button>
        </div>
    )
}

export function SelectionPanel() {
    const selected = useSelectionStore((s) => s.selected)
    const clear = useSelectionStore((s) => s.clear)

    return (
        <aside className="w-80 shrink-0 overflow-y-auto border-l border-border bg-card p-4">
            {selected.length === 0 && (
                <div className="flex h-full flex-col items-center justify-center gap-2 text-center text-sm text-muted-foreground">
                    <ImageIcon className="h-8 w-8"/>
                    <p>Select a photo to see its details.</p>
                    <p className="text-xs">Click to select · ⌘/Ctrl-click to multi-select · Shift-click for a range.</p>
                </div>
            )}
            {selected.length === 1 && <SinglePicture id={selected[0]}/>}
            {selected.length > 1 && <MultiSelection ids={selected} onClear={clear}/>}
        </aside>
    )
}
