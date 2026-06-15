import {useMemo} from 'react'
import {useNavigate} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {toast} from 'sonner'
import {ImageIcon, List, Pencil, Table2, Trash2, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {getPicture, getPictureUrl} from '@/api/pictures'
import {listPictureTagsWithSources} from '@/api/tags'
import {apiErrorMessage} from '@/api/client'
import {useBatchEditTags, usePictureTags} from '@/hooks/useTags'
import {useIncomingShares, useOutgoingShares} from '@/hooks/useShares'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {TagPicker} from '@/components/tags/TagPicker'
import {Section} from '@/components/photos/detail/Section'
import {ExifEditDialog} from '@/components/photos/detail/ExifEditDialog'
import {ShareStatusBadge} from '@/components/shares/ShareStatusBadge'
import {queryKeys} from '@/lib/constants'
import {TagPath} from '@/lib/utils'
import type {IncomingShareResponse, TagSource} from '@/lib/types'

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

function decodeLabel(label: string): string {
    return label.replace(/_AT_/g, '@').replace(/_DOT_/g, '.')
}

/** Resolve a `SharedToMe.<sender_AT_inst>.…` tag to its incoming share id. */
function incomingShareIdForTag(wire: string, shares: IncomingShareResponse[]): string | null {
    const seg = wire.split('.')
    if (seg[0] !== 'SharedToMe' || !seg[1]) return null
    const handle = decodeLabel(seg[1])
    return shares.find((s) => `${s.sender_username}@${s.sender_instance}` === handle)?.id ?? null
}

function InfoRow({label, value}: { label: string; value: string }) {
    return (
        <div className="flex items-baseline justify-between gap-2 text-sm">
            <span className="text-muted-foreground">{label}</span>
            <span className="truncate text-right">{value}</span>
        </div>
    )
}

const SOURCE_LABEL: Record<TagSource, string> = {
    manual: 'manual',
    rule: 'rule',
    segment: 'segment',
    share_mapping: 'mapping',
    incoming_share: 'share',
}

function TagChips({
                      tags,
                      onRemove,
                      onAdd,
                      onTagClick,
                  }: {
    tags: string[]
    onRemove: (wire: string) => void
    onAdd: (wire: string) => void
    onTagClick: (wire: string) => void
}) {
    return (
        <div className="space-y-2">
            <div className="flex flex-wrap gap-1.5">
                {tags.length > 0 ? (
                    tags.map((t) => (
                        <Badge key={t} variant="secondary" className="gap-1 font-normal">
                            <button onClick={() => onTagClick(t)} className="hover:text-primary">
                                {TagPath.toDisplay(t)}
                            </button>
                            <button onClick={() => onRemove(t)} aria-label={`Remove ${t}`} className="ml-0.5">
                                <X className="h-3 w-3"/>
                            </button>
                        </Badge>
                    ))
                ) : (
                    <span className="text-xs text-muted-foreground">No tags.</span>
                )}
            </div>
            <TagPicker onSelect={onAdd} excludePaths={tags}/>
        </div>
    )
}

function TagProvenanceTable({
                                rows,
                                onRemove,
                                onAdd,
                                onTagClick,
                                onSourceClick,
                                excludePaths,
                            }: {
    rows: Array<{ path: string; sources: Array<{ source: TagSource; source_id: string | null }> }>
    onRemove: (wire: string) => void
    onAdd: (wire: string) => void
    onTagClick: (wire: string) => void
    onSourceClick: (source: TagSource, sourceId: string | null) => void
    excludePaths: string[]
}) {
    return (
        <div className="space-y-2">
            <div className="space-y-1">
                {rows.length > 0 ? (
                    rows.map((row) => {
                        const removable = row.sources.some((s) => s.source === 'manual')
                        return (
                            <div key={row.path} className="flex items-center justify-between gap-2 text-sm">
                                <button onClick={() => onTagClick(row.path)} className="truncate text-left hover:text-primary">
                                    {TagPath.toDisplay(row.path)}
                                </button>
                                <div className="flex shrink-0 items-center gap-1">
                                    {row.sources.map((s, i) => {
                                        const clickable = !!s.source_id && s.source !== 'manual'
                                        return (
                                            <button
                                                key={`${s.source}-${s.source_id ?? i}`}
                                                onClick={() => clickable && onSourceClick(s.source, s.source_id)}
                                                disabled={!clickable}
                                                className="rounded bg-muted px-1 text-[10px] leading-4 text-muted-foreground enabled:hover:text-foreground"
                                                title={clickable ? 'View source' : undefined}
                                            >
                                                {SOURCE_LABEL[s.source]}
                                            </button>
                                        )
                                    })}
                                    {removable && (
                                        <button onClick={() => onRemove(row.path)} aria-label={`Remove ${row.path}`}>
                                            <X className="h-3 w-3 text-muted-foreground hover:text-foreground"/>
                                        </button>
                                    )}
                                </div>
                            </div>
                        )
                    })
                ) : (
                    <span className="text-xs text-muted-foreground">No tags.</span>
                )}
            </div>
            <TagPicker onSelect={onAdd} excludePaths={excludePaths}/>
        </div>
    )
}

function SinglePicture({id}: { id: string }) {
    const navigate = useNavigate()
    const {update} = useGalleryParams()
    const {data: picture, isPending} = useQuery({queryKey: queryKeys.picture(id), queryFn: () => getPicture(id)})
    const {data: preview} = useQuery({
        queryKey: ['pictures', 'url', id, 'medium'],
        queryFn: () => getPictureUrl(id, 'medium'),
        staleTime: 10 * 60 * 1000,
    })
    const {data: plainTags} = usePictureTags(id)
    const {data: outgoing} = useOutgoingShares()
    const {data: incoming} = useIncomingShares()

    const tagProvenance = useUIStore((s) => s.tagProvenance)
    const toggleTagProvenance = useUIStore((s) => s.toggleTagProvenance)

    const {data: provenance} = useQuery({
        queryKey: ['tags', 'detail', id, 'sources'],
        queryFn: () => listPictureTagsWithSources(id),
        enabled: tagProvenance,
    })

    const batch = useBatchEditTags()
    const add = (wire: string) =>
        batch.mutate(
            {picture_ids: [id], add_tags: [wire]},
            {onError: (e) => toast.error('Could not add tag', {description: apiErrorMessage(e)})},
        )
    const remove = (wire: string) =>
        batch.mutate(
            {picture_ids: [id], remove_tags: [wire]},
            {onError: (e) => toast.error('Could not remove tag', {description: apiErrorMessage(e)})},
        )

    const onTagClick = (wire: string) => update({tag: wire})
    const onSourceClick = (source: TagSource, sourceId: string | null) => {
        if (!sourceId) return
        if (source === 'incoming_share') update({panel: 'incoming', share: sourceId})
        else if (source !== 'manual') navigate(`/tagging/${sourceId}`)
    }
    const onSharedTagClick = (wire: string) =>
        update({tag: wire, panel: 'incoming', share: incomingShareIdForTag(wire, incoming ?? [])})

    const tags = plainTags ?? []
    const regularTags = useMemo(() => tags.filter((t) => !TagPath.isProtected(t)), [tags])
    const sharedToMeTags = useMemo(() => tags.filter((t) => TagPath.isProtected(t)), [tags])

    const relatedShares = useMemo(() => {
        const live = (outgoing ?? []).filter((s) => s.status !== 'revoked' && s.status !== 'tombstoned')
        return live.filter((s) => tags.some((t) => t === s.tag_path || t.startsWith(`${s.tag_path}.`)))
    }, [outgoing, tags])

    const provenanceRows = useMemo(
        () => (provenance?.tags ?? []).filter((r) => !TagPath.isProtected(r.path)),
        [provenance],
    )

    if (isPending || !picture) {
        return <div className="h-40 animate-pulse rounded-md bg-muted"/>
    }

    const owned = picture.owner_username == null

    const exifRows: Array<[string, string]> = []
    if (picture.orientation != null) exifRows.push(['Orientation', String(picture.orientation)])
    if (picture.gps_lat != null && picture.gps_lng != null)
        exifRows.push(['GPS', `${picture.gps_lat.toFixed(5)}, ${picture.gps_lng.toFixed(5)}`])
    if (picture.gps_alt != null) exifRows.push(['Altitude', `${picture.gps_alt} m`])
    for (const [k, v] of Object.entries(picture.exif_data ?? {})) {
        if (v == null || typeof v === 'object') continue
        exifRows.push([k, String(v)])
    }

    return (
        <div>
            <div className="overflow-hidden rounded-md bg-muted">
                {preview ? (
                    <img src={preview.url} alt={picture.filename ?? ''} className="max-h-56 w-full object-contain"/>
                ) : (
                    <div className="flex h-32 items-center justify-center text-muted-foreground">
                        <ImageIcon className="h-8 w-8"/>
                    </div>
                )}
            </div>

            <div className="mt-3">
                <p className="truncate text-sm font-medium" title={picture.filename ?? undefined}>
                    {picture.filename ?? 'Untitled'}
                </p>
                <p className="text-xs text-muted-foreground">
                    {formatBytes(picture.file_size)}
                    {picture.width && picture.height ? ` · ${picture.width} × ${picture.height}` : ''}
                </p>
            </div>

            <div className="mt-2">
                <Section
                    id="tags"
                    title="Tags"
                    count={regularTags.length}
                    action={
                        <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7"
                            onClick={toggleTagProvenance}
                            title={tagProvenance ? 'Show as list' : 'Show provenance'}
                        >
                            {tagProvenance ? <List className="h-3.5 w-3.5"/> : <Table2 className="h-3.5 w-3.5"/>}
                        </Button>
                    }
                >
                    {tagProvenance ? (
                        <TagProvenanceTable
                            rows={provenanceRows}
                            onRemove={remove}
                            onAdd={add}
                            onTagClick={onTagClick}
                            onSourceClick={onSourceClick}
                            excludePaths={regularTags}
                        />
                    ) : (
                        <TagChips tags={regularTags} onRemove={remove} onAdd={add} onTagClick={onTagClick}/>
                    )}
                </Section>

                {sharedToMeTags.length > 0 && (
                    <Section id="shared-with-you" title="Shared with you" count={sharedToMeTags.length} defaultOpen={false}>
                        <div className="space-y-1">
                            {sharedToMeTags.map((t) => (
                                <button
                                    key={t}
                                    onClick={() => onSharedTagClick(t)}
                                    className="block w-full truncate text-left text-xs text-muted-foreground hover:text-primary"
                                    title={TagPath.toDisplay(t)}
                                >
                                    {TagPath.toDisplay(t)}
                                </button>
                            ))}
                        </div>
                    </Section>
                )}

                {relatedShares.length > 0 && (
                    <Section id="shared-by-you" title="Shared by you" count={relatedShares.length} defaultOpen={false}>
                        <div className="max-h-48 space-y-1.5 overflow-y-auto pr-1">
                            {relatedShares.map((s) => (
                                <button
                                    key={s.id}
                                    onClick={() => update({tag: s.tag_path, panel: 'outgoing'})}
                                    className="flex w-full items-center justify-between gap-2 text-xs hover:text-primary"
                                >
                  <span className="min-w-0 truncate">
                    → @{s.recipient_username}:{s.recipient_instance}
                  </span>
                                    <ShareStatusBadge status={s.status}/>
                                </button>
                            ))}
            </div>
                    </Section>
                )}

                <Section id="details" title="Details">
                    <div className="space-y-1.5">
                        <InfoRow
                            label="Dimensions"
                            value={picture.width && picture.height ? `${picture.width} × ${picture.height}` : '—'}
                        />
                        <InfoRow label="Size" value={formatBytes(picture.file_size)}/>
                        <InfoRow label="Type" value={picture.mime_type ?? '—'}/>
                        <InfoRow label="Taken" value={formatDate(picture.captured_at)}/>
                        <InfoRow label="Added" value={formatDate(picture.ingested_at)}/>
                        {picture.owner_username && (
                            <InfoRow label="Owner" value={`@${picture.owner_username}:${picture.owner_instance_domain ?? '?'}`}/>
                        )}
                        <InfoRow label="EXIF sync" value={picture.exif_sync_status}/>
                    </div>
                </Section>

                <Section
                    id="exif"
                    title="EXIF"
                    defaultOpen={false}
                    action={
                        owned ? (
                            <ExifEditDialog picture={picture}/>
                        ) : (
                            <Button variant="ghost" size="icon" className="h-7 w-7" disabled title="Received pictures can't be edited">
                                <Pencil className="h-3.5 w-3.5"/>
                            </Button>
                        )
                    }
                >
                    {exifRows.length > 0 ? (
                        <div className="space-y-1.5">
                            {exifRows.map(([k, v]) => (
                                <InfoRow key={k} label={k} value={v}/>
                            ))}
                        </div>
                    ) : (
                        <span className="text-xs text-muted-foreground">No EXIF data.</span>
                    )}
                </Section>

                <Section id="versions" title="Versions" count={picture.versions.length} defaultOpen={false}>
                    {picture.versions.length > 0 ? (
                        <div className="space-y-1.5">
                            {picture.versions.map((v) => (
                                <InfoRow key={v.id} label={`v${v.version_number}`} value={formatDate(v.created_at)}/>
                            ))}
                        </div>
                    ) : (
                        <span className="text-xs text-muted-foreground">No previous versions.</span>
                    )}
                </Section>
            </div>
        </div>
    )
}

function MultiSelection({ids, onClear}: { ids: string[]; onClear: () => void }) {
    const batch = useBatchEditTags()
    const add = (wire: string) =>
        batch.mutate(
            {picture_ids: ids, add_tags: [wire]},
            {onError: (e) => toast.error('Could not add tag', {description: apiErrorMessage(e)})},
        )

    return (
        <div className="space-y-3">
            <p className="text-sm">
                <span className="font-medium">{ids.length}</span> photos selected
            </p>
            <Section id="multi-tags" title="Tag all selected">
                <TagPicker onSelect={add} triggerLabel="Add tag to all"/>
            </Section>
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
