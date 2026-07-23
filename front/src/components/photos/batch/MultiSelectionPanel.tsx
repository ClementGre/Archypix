import {useMemo, useState} from 'react'
import {ArchiveRestore, Loader2, Plus, Trash2, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {Section} from '@/components/photos/detail/Section'
import {TagPicker} from '@/components/tags/TagPicker'
import {BatchConfirmDialog} from './BatchConfirmDialog'
import {BatchExifSection, BatchMetadataSection} from './BatchExifSection'
import {BatchCreatorControl} from './BatchCreatorControl'
import {FixBulkSection} from '@/components/photos/fix/FixBulkSection'
import {batchRestore, batchTrash} from '@/api/pictures'
import {batchEditTags} from '@/api/tags'
import {useAggregate} from '@/hooks/useAggregate'
import {useBatchMutations} from '@/hooks/useBatch'
import {toApiSelection, useSelectionStore} from '@/stores/selection'
import {usePersistentBool} from '@/hooks/usePersistentBool'
import {cn, formatBytes, TagPath} from '@/lib/utils'
import type {AggregateSection, TagAggregate, TagSource} from '@/lib/types'

const SOURCE_LABEL: Record<TagSource, string> = {
    manual: 'manual',
    rule: 'rule',
    segment: 'segment',
    share_mapping: 'mapping',
    incoming_share: 'share',
}

const SOURCE_COLOR: Record<TagSource, string> = {
    manual: 'bg-blue-500/15 text-blue-400',
    rule: 'bg-emerald-500/15 text-emerald-400',
    segment: 'bg-violet-500/15 text-violet-400',
    share_mapping: 'bg-amber-500/15 text-amber-400',
    incoming_share: 'bg-sky-500/15 text-sky-400',
}

/** One tag per line: path chip (tristate count) + per-source mini-tags (wrap under it) carrying counts. */
function TagProvenanceRow({tag, total, selection, onRemove}: {
    tag: TagAggregate
    total: number
    selection: ReturnType<typeof toApiSelection>
    onRemove: () => void
}) {
    const [open, setOpen] = useState(false)
    const onAll = tag.count >= total
    const removable = tag.manual_count > 0
    const display = TagPath.toDisplay(tag.path)
    return (
        <div className="flex flex-wrap items-center gap-1">
            <Tooltip delayDuration={0}>
                <TooltipTrigger asChild>
                    <Badge
                        variant={onAll ? 'secondary' : 'outline'}
                        className={cn('min-w-0 max-w-full gap-1 font-normal', !onAll && 'border-dashed text-muted-foreground')}
                    >
                        <span className="truncate">{display}</span>
                        {!onAll && <span className="shrink-0 text-[10px] tabular-nums">{tag.count}/{total}</span>}
                        {removable && (
                            <button
                                onClick={() => setOpen(true)}
                                aria-label={`Remove ${tag.path}`}
                                className="-mr-0.5 ml-0.5 shrink-0 rounded p-0.5 hover:bg-foreground/20"
                            >
                                <X className="h-3 w-3"/>
                            </button>
                        )}
                    </Badge>
                </TooltipTrigger>
                <TooltipContent className="max-w-[16rem] break-all text-xs">{display}</TooltipContent>
            </Tooltip>

            {(tag.sources ?? []).map((s, i) => (
                <span
                    key={`${s.source}-${i}`}
                    className={cn('flex items-center gap-1 rounded px-1 text-[10px] font-medium leading-4', SOURCE_COLOR[s.source])}
                    title={`${s.count} via ${SOURCE_LABEL[s.source]}`}
                >
                    {SOURCE_LABEL[s.source]}
                    <span className="tabular-nums opacity-80">{s.count}</span>
                </span>
            ))}

            {removable && (
                <BatchConfirmDialog
                    open={open}
                    onOpenChange={setOpen}
                    title={`Remove "${display}"?`}
                    description="Only manual tags are removed — tags assigned by rules, segments or shares are left untouched."
                    confirmLabel="Remove tag"
                    destructive
                    dryRun={() => batchEditTags({selection, remove_tags: [tag.path], dry_run: true})}
                    renderResult={(r) => (
                        <span>
                            Removes from <span className="font-medium tabular-nums">{r.removed ?? r.affected}</span> of {total} photos.
                        </span>
                    )}
                    onConfirm={onRemove}
                />
            )}
        </div>
    )
}

export function MultiSelectionPanel() {
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const clear = useSelectionStore((s) => s.clear)

    const selection = useMemo(() => toApiSelection({query, includeIds, excludeIds}), [query, includeIds, excludeIds])

    const {trash, restore, tags: tagsMutation} = useBatchMutations()

    // Per-section laziness (§4): only request `tags`/`exif` when their panel is open. The Info and
    // EXIF sections both read the `exif` aggregate.
    const [tagsOpen, setTagsOpen] = usePersistentBool('multi-tags', true)
    const [metaOpen, setMetaOpen] = usePersistentBool('multi-meta', false)
    const [exifOpen, setExifOpen] = usePersistentBool('multi-exif', false)

    const sections = useMemo<AggregateSection[]>(() => {
        const s: AggregateSection[] = ['summary']
        if (tagsOpen) s.push('tags')
        if (metaOpen || exifOpen) s.push('exif')
        return s
    }, [tagsOpen, metaOpen, exifOpen])

    const {data: agg, isPending} = useAggregate(selection, sections, {
        tagProvenance: true,
        // Poll while a batch EXIF edit drains so the convergence count ticks down (§6.3).
        refetchInterval: (q) => {
            const d = q.state.data
            const busy = (d?.exif_sync.pending ?? 0) + (d?.exif_sync.pending_job_creation ?? 0)
            return busy > 0 ? 4000 : false
        },
    })

    const total = agg?.count ?? 0
    const trashedCount = agg?.trashed_count ?? 0
    const inFlight = (agg?.exif_sync.pending ?? 0) + (agg?.exif_sync.pending_job_creation ?? 0)
    const hasReceived = (agg?.received_count ?? 0) > 0

    const [pendingAdd, setPendingAdd] = useState<string | null>(null)

    return (
        <div className="px-3 py-2">
            {/* Header */}
            <div className="flex items-center justify-between gap-2 pb-2">
                <p className="text-sm">
                    {isPending ? (
                        <span className="text-muted-foreground">Loading…</span>
                    ) : (
                        <>
                            <span className="font-medium tabular-nums">{total}</span> photos selected
                        </>
                    )}
                </p>
                <Button variant="ghost" size="sm" className="h-7 gap-1.5 text-muted-foreground" onClick={clear}>
                    <X className="h-3.5 w-3.5"/> Clear
                </Button>
            </div>

            {/* Batch trash / restore — same line (wrap if tight). Disabled when nothing would change. */}
            <div className="flex flex-wrap gap-2 pb-2">
                <BatchConfirmDialog
                    trigger={
                        <Button
                            variant="outline"
                            className="min-w-[8rem] flex-1 justify-center gap-2 text-destructive"
                            disabled={trash.isPending || total - trashedCount === 0}
                        >
                            <Trash2 className="h-4 w-4"/> Trash
                        </Button>
                    }
                    title="Move selection to trash?"
                    description="Owned photos are purged after your retention window; received photos are only hidden locally."
                    confirmLabel="Move to trash"
                    destructive
                    dryRun={() => batchTrash(selection, true)}
                    onConfirm={() => trash.mutate(selection, {onSuccess: clear})}
                />
                <BatchConfirmDialog
                    trigger={
                        <Button
                            variant="outline"
                            className="min-w-[8rem] flex-1 justify-center gap-2"
                            disabled={restore.isPending || trashedCount === 0}
                        >
                            <ArchiveRestore className="h-4 w-4"/> Restore
                        </Button>
                    }
                    title="Restore selection?"
                    description="Clears the trash flag on every selected photo that is currently trashed."
                    confirmLabel="Restore"
                    dryRun={() => batchRestore(selection, true)}
                    onConfirm={() => restore.mutate(selection, {onSuccess: clear})}
                />
            </div>

            {/* Summary */}
            <Section id="multi-summary" title="Summary">
                {agg ? (
                    <div className="space-y-1 text-sm">
                        <SummaryRow label="Owned" value={`${agg.owned_count}`}/>
                        <SummaryRow label="Received" value={`${agg.received_count}`}/>
                        <SummaryRow label="Total size" value={formatBytes(agg.total_file_size)}/>
                        {agg.trashed_count > 0 && <SummaryRow label="In trash" value={`${agg.trashed_count}`}/>}
                        {agg.owner_deleting_count > 0 && <SummaryRow label="Owner deleting" value={`${agg.owner_deleting_count}`}/>}
                        {agg.duplicate_count > 0 && <SummaryRow label="Duplicates" value={`${agg.duplicate_count}`}/>}
                        {agg.thumbnail_pending_count > 0 && <SummaryRow label="Thumbnails pending" value={`${agg.thumbnail_pending_count}`}/>}
                        {agg.owners.length > 0 && (
                            <div className="pt-1">
                                <p className="text-xs text-muted-foreground">Owners</p>
                                {agg.owners.map((o) => (
                                    <SummaryRow key={`${o.username}@${o.instance}`} label={`@${o.username}:${o.instance}`} value={`${o.count}`}/>
                                ))}
                            </div>
                        )}
                        <BatchCreatorControl creator={agg.creator} total={total} selection={selection}/>
                        {inFlight > 0 && (
                            <p className="flex items-center gap-1.5 pt-1 text-xs text-muted-foreground">
                                <Loader2 className="h-3 w-3 animate-spin"/>
                                {inFlight} EXIF {inFlight === 1 ? 'edit' : 'edits'} syncing to files…
                            </p>
                        )}
                    </div>
                ) : (
                    <div className="h-16 animate-pulse rounded bg-muted"/>
                )}
            </Section>

            {/* Fix-tools bulk section (feature 30) — shown before Tags when fix mode is on. */}
            <FixBulkSection/>

            {/* Tags (tristate, with per-source provenance counts) */}
            <Section
                id="multi-tags"
                title="Tags"
                open={tagsOpen}
                onOpenChange={setTagsOpen}
                action={
                    <TagPicker
                        onSelect={(wire) => setPendingAdd(wire)}
                        trigger={
                            <Button variant="ghost" size="icon" className="h-7 w-7" title="Add tag to all">
                                <Plus className="h-3.5 w-3.5"/>
                            </Button>
                        }
                    />
                }
            >
                {!agg?.tags ? (
                    <div className="h-6 animate-pulse rounded bg-muted"/>
                ) : total === 0 || agg.tags.filter((t) => !TagPath.isProtected(t.path)).length === 0 ? (
                    <span className="text-xs text-muted-foreground">No tags.</span>
                ) : (
                    <div className="space-y-1.5">
                        {agg.tags
                            .filter((t) => !TagPath.isProtected(t.path))
                            .map((t) => (
                                <TagProvenanceRow
                                    key={t.path}
                                    tag={t}
                                    total={total}
                                    selection={selection}
                                    onRemove={() => tagsMutation.mutate({selection, remove_tags: [t.path]})}
                                />
                            ))}
                    </div>
                )}
            </Section>

            {/* Read-only file/metadata aggregates */}
            <BatchMetadataSection exif={agg?.exif} total={total} open={metaOpen} onOpenChange={setMetaOpen}/>

            {/* EXIF — inline-editable aggregate (dry-run computed when the confirm popup opens) */}
            <BatchExifSection exif={agg?.exif} total={total} selection={selection} hasReceived={hasReceived} open={exifOpen}
                              onOpenChange={setExifOpen}/>

            {/* Confirm for an added tag (TagPicker picks, then we confirm + dry-run). */}
            {pendingAdd && (
                <BatchConfirmDialog
                    open
                    onOpenChange={(o) => !o && setPendingAdd(null)}
                    title={`Add "${TagPath.toDisplay(pendingAdd)}"?`}
                    description="Adds this tag (as a manual tag) to the selected photos that don't already have it."
                    confirmLabel="Add tag"
                    dryRun={() => batchEditTags({selection, add_tags: [pendingAdd], dry_run: true})}
                    renderResult={(r) => (
                        <span>
                            Adds to <span className="font-medium tabular-nums">{r.added ?? r.affected}</span> of {total} photos.
                        </span>
                    )}
                    onConfirm={() => {
                        tagsMutation.mutate({selection, add_tags: [pendingAdd]})
                        setPendingAdd(null)
                    }}
                />
            )}
        </div>
    )
}

function SummaryRow({label, value}: { label: string; value: string }) {
    return (
        <div className="flex items-baseline justify-between gap-3">
            <span className="min-w-0 truncate text-muted-foreground">{label}</span>
            <span className="tabular-nums">{value}</span>
        </div>
    )
}
