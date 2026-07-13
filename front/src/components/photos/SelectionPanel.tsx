import {useEffect, useMemo, useState} from 'react'
import {useNavigate, useSearchParams} from 'react-router-dom'
import {useQuery, useQueryClient} from '@tanstack/react-query'
import {toast} from 'sonner'
import {AlertTriangle, ArchiveRestore, Copy, Download, ImageIcon, List, Loader2, Plus, RotateCcw, RotateCw, Table2, Trash2, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {downloadOriginal, getPicture, getPictureUrl} from '@/api/pictures'
import {listPictureTagsWithSources} from '@/api/tags'
import {apiErrorMessage} from '@/api/client'
import {useBatchEditTags, usePictureTags} from '@/hooks/useTags'
import {useCopyPicture, useTrashMutations} from '@/hooks/usePictureEdit'
import {useIncomingShares, useOutgoingShares} from '@/hooks/useShares'
import {useSettings} from '@/hooks/useSettings'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {bestLoaded, recordImage, useImageCache, VARIANT_RANK} from '@/stores/imageCache'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {TagPicker} from '@/components/tags/TagPicker'
import {Section} from '@/components/photos/detail/Section'
import {CreatorField} from '@/components/photos/detail/CreatorField'
import {CopiesSection} from '@/components/photos/detail/CopiesSection'
import {OverwrittenBadge} from '@/components/photos/detail/OverwrittenBadge'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {displayDimensions, OrientedContainImage} from '@/components/photos/OrientedImage'
import {FileTypeIcon} from '@/components/photos/FileTypeIcon'
import {PlayBadge} from '@/components/photos/PlayBadge'
import {MediaPlayer} from '@/components/photos/MediaPlayer'
import {ExifInlineEditor} from '@/components/photos/detail/ExifInlineEditor'
import {MultiSelectionPanel} from '@/components/photos/batch/MultiSelectionPanel'
import {useExifDraft} from '@/hooks/useExifDraft'
import {ShareStatusBadge} from '@/components/shares/ShareStatusBadge'
import {queryKeys} from '@/lib/constants'
import {cn, formatBytes, formatDateTime, isAudioMime, isVideoMime, TagPath, variantForSize} from '@/lib/utils'
import {deadlineLabel, ownedPurgeAt} from '@/lib/trash'
import type {IncomingShareResponse, PictureDetail, TagSource} from '@/lib/types'

// ── Helpers ───────────────────────────────────────────────────────────────────

function decodeLabel(label: string): string {
    return label.replace(/_AT_/g, '@').replace(/_DOT_/g, '.')
}

/** Format a decoded `alice@ex.com` handle as `@alice:ex.com`. */
function formatHandle(handle: string): string {
    const [username, ...rest] = handle.split('@')
    return rest.length ? `@${username}:${rest.join('@')}` : `@${handle}`
}

/** Decompose a `SharedToMe.<handle>.<sub.path>` tag into its sender + shared subpath. */
function parseSharedTag(wire: string): { handle: string; subpath: string } {
    const seg = wire.split('.')
    return {
        handle: seg[1] ? formatHandle(decodeLabel(seg[1])) : 'Unknown',
        subpath: seg.length > 2 ? TagPath.toDisplay(seg.slice(2).join('.')) : '/',
    }
}

function incomingShareIdForTag(wire: string, shares: IncomingShareResponse[]): string | null {
    const seg = wire.split('.')
    if (seg[0] !== 'SharedToMe' || !seg[1]) return null
    const handle = decodeLabel(seg[1])
    return shares.find((s) => `${s.sender_username}@${s.sender_instance}` === handle)?.id ?? null
}

const SOURCE_LABEL: Record<TagSource, string> = {
    manual: 'manual',
    rule: 'rule',
    segment: 'segment',
    share_mapping: 'mapping',
    incoming_share: 'share',
}

/** Per-source accent colour for the small provenance tags. */
const SOURCE_COLOR: Record<TagSource, string> = {
    manual: 'bg-blue-500/15 text-blue-400',
    rule: 'bg-emerald-500/15 text-emerald-400',
    segment: 'bg-violet-500/15 text-violet-400',
    share_mapping: 'bg-amber-500/15 text-amber-400',
    incoming_share: 'bg-sky-500/15 text-sky-400',
}

// ── Tag components ────────────────────────────────────────────────────────────

function TagChip({wire, onRemove, onTagClick}: { wire: string; onRemove?: () => void; onTagClick: () => void }) {
    const display = TagPath.toDisplay(wire)
    return (
        <Badge variant="secondary" className="min-w-0 max-w-full gap-1 font-normal">
            <Tooltip delayDuration={0}>
                <TooltipTrigger asChild>
                    <button onClick={onTagClick} className="truncate hover:text-primary">
                        {display}
                    </button>
                </TooltipTrigger>
                <TooltipContent className="max-w-[16rem] break-all text-xs">{display}</TooltipContent>
            </Tooltip>
            {onRemove && (
                <button onClick={onRemove} aria-label={`Remove ${wire}`} className="-mr-0.5 ml-0.5 shrink-0 rounded p-0.5 hover:bg-foreground/20">
                    <X className="h-3 w-3"/>
                </button>
            )}
        </Badge>
    )
}

function TagChips({tags, onRemove, onTagClick}: {
    tags: string[]
    onRemove: (wire: string) => void
    onTagClick: (wire: string) => void
}) {
    if (tags.length === 0) return <span className="text-xs text-muted-foreground">No tags.</span>
    return (
        <div className="flex flex-wrap gap-1.5">
            {tags.map((t) => (
                <TagChip key={t} wire={t} onRemove={() => onRemove(t)} onTagClick={() => onTagClick(t)}/>
            ))}
        </div>
    )
}

function TagProvenanceTable({rows, onRemove, onTagClick, onSourceClick}: {
    rows: Array<{ path: string; sources: Array<{ source: TagSource; source_id: string | null }> }>
    onRemove: (wire: string) => void
    onTagClick: (wire: string) => void
    onSourceClick: (source: TagSource, sourceId: string | null) => void
}) {
    if (rows.length === 0) return <span className="text-xs text-muted-foreground">No tags.</span>
    return (
        <div className="space-y-1.5">
            {rows.map((row) => {
                const removable = row.sources.some((s) => s.source === 'manual')
                return (
                    <div key={row.path} className="flex flex-wrap items-center gap-1">
                        <TagChip
                            wire={row.path}
                            onRemove={removable ? () => onRemove(row.path) : undefined}
                            onTagClick={() => onTagClick(row.path)}
                        />
                        <div className="flex flex-wrap items-center gap-1">
                            {row.sources.map((s, i) => {
                                const clickable = !!s.source_id && s.source !== 'manual'
                                return (
                                    <button
                                        key={`${s.source}-${s.source_id ?? i}`}
                                        onClick={() => clickable && onSourceClick(s.source, s.source_id)}
                                        disabled={!clickable}
                                        className={cn(
                                            'rounded px-1 text-[10px] font-medium leading-4',
                                            SOURCE_COLOR[s.source],
                                            clickable && 'hover:brightness-125',
                                        )}
                                        title={clickable ? 'View source' : undefined}
                                    >
                                        {SOURCE_LABEL[s.source]}
                                    </button>
                                )
                            })}
                        </div>
                    </div>
                )
            })}
        </div>
    )
}

// ── Single picture ────────────────────────────────────────────────────────────

/** Max height (px) of the sidebar preview image; also drives the requested thumbnail variant. */
const PREVIEW_MAX_HEIGHT = 208

function SinglePicture({id}: { id: string }) {
    const {data: picture, isPending} = useQuery({
        queryKey: queryKeys.picture(id),
        queryFn: () => getPicture(id),
    })

    if (isPending || !picture) {
        return <div className="mx-3 mt-3 h-40 animate-pulse rounded-md bg-muted"/>
    }
    return <PictureBody id={id} picture={picture}/>
}

function PictureBody({id, picture}: { id: string; picture: PictureDetail }) {
    const navigate = useNavigate()
    const [, setSp] = useSearchParams()
    const {update} = useGalleryParams()

    const isVideo = isVideoMime(picture.mime_type)
    const isAudio = isAudioMime(picture.mime_type)
    const isMedia = isVideo || isAudio

    // Size the preview to its capped display height (the preview never exceeds PREVIEW_MAX_HEIGHT,
    // so the sidebar width doesn't matter); the lightbox always uses `large`.
    const previewVariant = variantForSize(PREVIEW_MAX_HEIGHT)
    // Reuse a higher-or-equal variant the browser already loaded (e.g. the lightbox's large image)
    // instead of a fresh medium presign; use a lower loaded variant as a progressive placeholder.
    const entry = useImageCache((s) => s.entries[id])
    const cached = useMemo(() => bestLoaded(entry), [entry])
    const reuseCached = !!cached && VARIANT_RANK[cached.variant] >= VARIANT_RANK[previewVariant]
    // Images and videos both have thumbnails (video's is a frame-grab); audio has none.
    const {data: preview} = useQuery({
        queryKey: ['pictures', 'url', id, previewVariant],
        queryFn: () => getPictureUrl(id, previewVariant),
        enabled: !isAudio && !reuseCached,
        staleTime: 10 * 60 * 1000,
    })
    const previewUrl = reuseCached ? cached!.url : preview?.url
    const previewUsedVariant = reuseCached ? cached!.variant : previewVariant

    // Audio plays inline in the panel from the original file; video opens the (autoplaying) Lightbox.
    const {data: mediaUrl} = useQuery({
        queryKey: ['pictures', 'url', id, 'original'],
        queryFn: () => getPictureUrl(id, 'original'),
        enabled: isAudio,
        staleTime: 10 * 60 * 1000,
    })

    const [downloading, setDownloading] = useState(false)
    const download = async () => {
        setDownloading(true)
        try {
            await downloadOriginal(id, picture.filename)
        } catch (e) {
            toast.error('Could not download', {description: apiErrorMessage(e)})
        } finally {
            setDownloading(false)
        }
    }
    const {data: plainTags} = usePictureTags(id)
    const {data: outgoing} = useOutgoingShares()
    const {data: incoming} = useIncomingShares()

    // For a received picture, find the live incoming share from its owner: a direct share's sender
    // IS the picture's owner. We match owner → sender to read the `allow_exif_edit` grant that gates
    // the "Suggest to owner" action. (Assumption: for transitive shares where the relayer differs
    // from the owner, no matching incoming share is found and only the local override is offered —
    // the propose path is owner-addressed anyway.)
    const incomingShare = useMemo(() => {
        if (picture.owner_username == null) return null
        const live = (incoming ?? []).filter((s) => s.status === 'active' || s.status === 'pending')
        return (
            live.find(
                (s) =>
                    s.sender_username === picture.owner_username &&
                    s.sender_instance === picture.owner_instance_domain,
            ) ?? null
        )
    }, [incoming, picture.owner_username, picture.owner_instance_domain])

    const exif = useExifDraft(picture, {allowExifEdit: !!incomingShare?.allow_exif_edit})

    const tagProvenance = useUIStore((s) => s.tagProvenance)
    const toggleTagProvenance = useUIStore((s) => s.toggleTagProvenance)
    const setLeftOpen = useUIStore((s) => s.setLeftOpen)
    const toggleMobileDrawer = useUIStore((s) => s.toggleMobileDrawer)
    const isMobile = useIsMobile()

    // Surface the left panel for a cross-link: dock it open on desktop, or switch the
    // mobile overlay from this (right) drawer to the left one.
    const revealLeftPanel = () => (isMobile ? toggleMobileDrawer('left') : setLeftOpen(true))

    const {data: provenance} = useQuery({
        queryKey: ['tags', 'detail', id, 'sources'],
        queryFn: () => listPictureTagsWithSources(id),
        enabled: tagProvenance,
    })

    const {trash, restore} = useTrashMutations()
    const copy = useCopyPicture()
    const {data: settings} = useSettings()

    const batch = useBatchEditTags()
    const addTag = (wire: string) =>
        batch.mutate(
            {picture_ids: [id], add_tags: [wire]},
            {onError: (e) => toast.error('Could not add tag', {description: apiErrorMessage(e)})},
        )
    const removeTag = (wire: string) =>
        batch.mutate(
            {picture_ids: [id], remove_tags: [wire]},
            {onError: (e) => toast.error('Could not remove tag', {description: apiErrorMessage(e)})},
        )

    // Clicking a tag filters by it and reveals it in the Tags tree (opens the left panel + tab).
    const onTagClick = (wire: string) => {
        revealLeftPanel()
        update({tag: wire, include: [], exclude: [], exact: [], panel: 'tags'})
    }
    const onSourceClick = (source: TagSource, sourceId: string | null) => {
        if (!sourceId) return
        if (source === 'incoming_share') {
            revealLeftPanel()
            update({panel: 'incoming', share: sourceId})
        } else if (source !== 'manual') navigate(`/tagging/${sourceId}`)
    }
    const onSharedTagClick = (wire: string) => {
        revealLeftPanel()
        update({tag: wire, include: [], exclude: [], exact: [], panel: 'incoming', share: incomingShareIdForTag(wire, incoming ?? [])})
    }

    const openLightbox = () =>
        setSp((prev) => {
            const next = new URLSearchParams(prev)
            next.set('view', id)
            return next
        })

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

    const owned = picture.owner_username == null
    const isTrashed = !!picture.deleted_at
    const ownerDeleted = !owned && !!picture.owner_deleted_at
    const ownerPurgeLabel = picture.owner_purge_at ? deadlineLabel(picture.owner_purge_at) : null
    // Owned trashed pictures: purge deadline is derived (deleted_at + retention).
    const ownedPurgeLabel =
        owned && isTrashed && settings
            ? deadlineLabel(ownedPurgeAt(picture.deleted_at, settings.trash_retention_days))
            : null
    const orientationOverridden = !owned && !!picture.local_exif_overrides && 'orientation' in picture.local_exif_overrides

    // Live orientation: the served thumbnail is raw pixels, so rotate by the current
    // (draft) orientation. Rotate buttons update the draft and auto-commit after a debounce.
    const draftOrientation = exif.draft.orientation ? Number(exif.draft.orientation) : 1
    const dispDims = displayDimensions(picture.width, picture.height, draftOrientation)

    return (
        <div>
            {isAudio ? (
                /* Audio plays inline in the panel — there's nothing to view full-screen. */
                <div className="px-3 pt-3">
                    {mediaUrl?.url ? (
                        <MediaPlayer src={mediaUrl.url} mime={picture.mime_type} title={picture.filename}/>
                    ) : (
                        <div className="flex h-16 items-center justify-center rounded-md bg-muted text-muted-foreground">
                            <Loader2 className="h-5 w-5 animate-spin"/>
                        </div>
                    )}
                </div>
            ) : (
                /* Thumbnail / video poster — borderless, full width, click opens lightbox */
                <div
                    className="group relative w-full cursor-pointer overflow-hidden bg-muted"
                    onClick={openLightbox}
                    title="Open full screen"
                >
                    {isVideo && previewUrl ? (
                        // Frame-grab thumbnail poster + play badge; playback happens in the Lightbox.
                        <div className="relative flex items-center justify-center bg-black">
                            <img
                                src={previewUrl}
                                alt={picture.filename ?? ''}
                                className="max-h-52 w-full object-contain"
                                onLoad={() => recordImage(id, previewUsedVariant, previewUrl, true)}
                            />
                            <PlayBadge hover/>
                        </div>
                    ) : previewUrl ? (
                        <OrientedContainImage
                            src={previewUrl}
                            alt={picture.filename ?? ''}
                            orientation={draftOrientation}
                            width={picture.width}
                            height={picture.height}
                            maxHeight={PREVIEW_MAX_HEIGHT}
                            placeholderSrc={cached && cached.url !== previewUrl ? cached.url : undefined}
                            onLoad={() => recordImage(id, previewUsedVariant, previewUrl, true)}
                        />
                    ) : (
                        // No thumbnail (pending, or a non-thumbnailable format) — show a file-type icon.
                        <div className="flex h-40 items-center justify-center text-muted-foreground">
                            <FileTypeIcon mime={picture.mime_type} filename={picture.filename} className="h-12 w-12 opacity-70"/>
                        </div>
                    )}

                    {/* Owner label for received pictures — overlaid on the preview. Turns red with a
                        tooltip when the owner has trashed the picture (grace-window). */}
                    {picture.owner_username && (
                        ownerDeleted ? (
                            <Tooltip>
                                <TooltipTrigger asChild>
                                    <span
                                        className="absolute bottom-2 left-2 flex items-center gap-1 rounded bg-destructive/85 px-1.5 py-0.5 text-[11px] font-medium text-white"
                                        onClick={(e) => e.stopPropagation()}
                                    >
                                        <AlertTriangle className="h-3 w-3"/>
                                        @{picture.owner_username}:{picture.owner_instance_domain ?? '?'}
                                    </span>
                                </TooltipTrigger>
                                <TooltipContent side="top" className="max-w-[15rem] text-xs">
                                    The owner moved this picture to their trash. It will be permanently removed
                                    {ownerPurgeLabel ? ` on ${ownerPurgeLabel}` : ' after their retention window'}.
                                </TooltipContent>
                            </Tooltip>
                        ) : (
                            <span className="absolute bottom-2 left-2 rounded bg-black/55 px-1.5 py-0.5 text-[11px] font-medium text-white">
                                @{picture.owner_username}:{picture.owner_instance_domain ?? '?'}
                            </span>
                        )
                    )}

                    {/* Rotate overlays — images only (EXIF orientation doesn't apply to video). Owned
                        pictures rotate their own EXIF; received pictures get a recipient-local override. */}
                    {!isMedia && (
                        <div className="absolute bottom-2 right-2 flex items-center gap-1 opacity-40 transition-opacity group-hover:opacity-100">
                            {orientationOverridden && (
                                <span onClick={(e) => e.stopPropagation()}>
                                    <OverwrittenBadge onRemove={() => exif.removeOverride('orientation')}/>
                                </span>
                            )}
                            <button
                                onClick={(e) => {
                                    e.stopPropagation();
                                    exif.rotate('ccw')
                                }}
                                title="Rotate left"
                                className="flex h-7 w-7 items-center justify-center rounded-md bg-black/60 text-white hover:bg-black/80"
                            >
                                <RotateCcw className="h-4 w-4"/>
                            </button>
                            <button
                                onClick={(e) => {
                                    e.stopPropagation();
                                    exif.rotate('cw')
                                }}
                                title="Rotate right"
                                className="flex h-7 w-7 items-center justify-center rounded-md bg-black/60 text-white hover:bg-black/80"
                            >
                                <RotateCw className="h-4 w-4"/>
                            </button>
                        </div>
                    )}
                </div>
            )}

            {/* Picture info */}
            <div className="flex items-start justify-between gap-1 px-3 pt-2 pb-1">
                <div className="min-w-0">
                    <p className="truncate text-sm font-medium" title={picture.filename ?? undefined}>
                        {picture.filename ?? 'Untitled'}
                    </p>
                    <p className="text-xs text-muted-foreground">
                        {[
                            formatBytes(picture.file_size),
                            dispDims.width && dispDims.height ? `${dispDims.width} × ${dispDims.height}` : null,
                            picture.mime_type,
                        ]
                            .filter(Boolean)
                            .join(' · ')}
                    </p>
                </div>
                <div className="flex shrink-0 items-center">
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-muted-foreground hover:text-foreground"
                        title="Download original"
                        disabled={downloading}
                        onClick={download}
                    >
                        {downloading ? <Loader2 className="h-4 w-4 animate-spin"/> : <Download className="h-4 w-4"/>}
                    </Button>
                    {/* Copy ("rescue") a received picture into your own library (feature 11). */}
                    {!owned && (
                        <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7 text-muted-foreground hover:text-foreground"
                            title="Copy to my library"
                            disabled={copy.isPending}
                            onClick={() => copy.mutate(id)}
                        >
                            {copy.isPending ? <Loader2 className="h-4 w-4 animate-spin"/> : <Copy className="h-4 w-4"/>}
                        </Button>
                    )}
                    {!isTrashed && (
                        <ConfirmDialog
                            trigger={
                                <Button
                                    variant="ghost"
                                    size="icon"
                                    className="h-7 w-7 text-muted-foreground hover:text-destructive"
                                    title="Move to trash"
                                >
                                    <Trash2 className="h-4 w-4"/>
                                </Button>
                            }
                            title="Move to trash?"
                            description={
                                owned
                                    ? 'This photo will be hidden and permanently deleted after your retention window. Shared recipients see a deletion warning until then.'
                                    : 'This removes the photo from your library locally. The owner\'s copy is unaffected.'
                            }
                            confirmLabel="Move to trash"
                            destructive
                            onConfirm={() => trash.mutate(id)}
                        />
                    )}
                </div>
            </div>

            {/* Timestamps above tags */}
            <div className="flex flex-col gap-0.5 px-3 pb-2 text-xs text-muted-foreground">
                <span>Added {formatDateTime(picture.ingested_at)}</span>
                <span>Edited {formatDateTime(picture.updated_at)}</span>
            </div>

            {/* Creator attribution (feature 26) — owned pictures edit the authoritative credit,
                received pictures a recipient-local override. */}
            <CreatorField picture={picture}/>

            {/* Owner-deletion grace-window warning (received pictures). */}
            {ownerDeleted && (
                <div
                    className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-2 text-xs text-destructive">
                    <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
                    <div className="flex flex-col gap-1.5">
                        <span>
                            The owner deleted this picture. It will disappear
                            {ownerPurgeLabel ? <> on <span
                                className="font-medium">{ownerPurgeLabel}</span></> : ' after their retention window'} unless they restore it.
                        </span>
                        <Button
                            variant="outline"
                            size="sm"
                            className="h-7 gap-1.5 self-start"
                            disabled={copy.isPending}
                            onClick={() => copy.mutate(id)}
                            title="Keep a permanent copy in your own library"
                        >
                            {copy.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <Copy className="h-3.5 w-3.5"/>}
                            Rescue to my library
                        </Button>
                    </div>
                </div>
            )}

            {/* Copy provenance — shown when this owned picture is a physical copy (feature 11). */}
            {owned && picture.copy_source_owner_username && (
                <div className="mx-3 mb-2 flex items-center gap-1.5 text-xs text-muted-foreground">
                    <Copy className="h-3.5 w-3.5 shrink-0"/>
                    <span>
                        Copy of @{picture.copy_source_owner_username}
                        {picture.copy_source_owner_instance ? `:${picture.copy_source_owner_instance}` : ''}
                    </span>
                </div>
            )}

            {/* Local-trash banner (this picture is in your trash). */}
            {isTrashed && (
                <div
                    className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-border bg-muted/50 px-2.5 py-2 text-xs text-muted-foreground">
                    <Trash2 className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
                    <span>
                        In your trash since {formatDateTime(picture.deleted_at)}.
                        {ownedPurgeLabel && <> Permanently deleted <span className="font-medium">{ownedPurgeLabel}</span>.</>}
                        {!owned && ' This only hides it locally — the owner\'s copy is untouched.'}
                    </span>
                </div>
            )}

            {/* Restore action (trashed pictures) */}
            {isTrashed && (
                <div className="px-3 pb-2">
                    <Button
                        variant="outline"
                        size="sm"
                        className="w-full justify-start gap-2"
                        disabled={restore.isPending}
                        onClick={() => restore.mutate(id)}
                    >
                        <ArchiveRestore className="h-4 w-4"/> Restore
                    </Button>
                </div>
            )}

            {/* Sections */}
            <div className="px-3">
                <Section
                    id="tags"
                    title="Tags"
                    count={regularTags.length}
                    action={
                        <div className="flex items-center gap-0.5">
                            <TagPicker
                                onSelect={addTag}
                                excludePaths={regularTags}
                                trigger={
                                    <Button variant="ghost" size="icon" className="h-7 w-7" title="Add tag">
                                        <Plus className="h-3.5 w-3.5"/>
                                    </Button>
                                }
                            />
                            <Button
                                variant="ghost"
                                size="icon"
                                className="h-7 w-7"
                                onClick={toggleTagProvenance}
                                title={tagProvenance ? 'Show as list' : 'Show provenance'}
                            >
                                {tagProvenance ? <List className="h-3.5 w-3.5"/> : <Table2 className="h-3.5 w-3.5"/>}
                            </Button>
                        </div>
                    }
                >
                    {tagProvenance ? (
                        <TagProvenanceTable
                            rows={provenanceRows}
                            onRemove={removeTag}
                            onTagClick={onTagClick}
                            onSourceClick={onSourceClick}
                        />
                    ) : (
                        <TagChips tags={regularTags} onRemove={removeTag} onTagClick={onTagClick}/>
                    )}
                </Section>

                {sharedToMeTags.length > 0 && (
                    <Section id="shared-with-you" title="Shared with you" count={sharedToMeTags.length} defaultOpen={false}>
                        <div className="max-h-56 space-y-1.5 overflow-y-auto pr-1">
                            {sharedToMeTags.map((t) => {
                                const {handle, subpath} = parseSharedTag(t)
                                return (
                                    <button
                                        key={t}
                                        onClick={() => onSharedTagClick(t)}
                                        className="flex w-full flex-col gap-0.5 rounded-md border border-border px-2 py-1.5 text-left transition-colors hover:border-primary/50"
                                    >
                                        <span className="truncate text-xs font-medium" title={handle}>{handle}</span>
                                        <span className="truncate text-[11px] text-muted-foreground" title={subpath}>{subpath}</span>
                                    </button>
                                )
                            })}
                        </div>
                    </Section>
                )}

                {relatedShares.length > 0 && (
                    <Section id="shared-by-you" title="Shared by you" count={relatedShares.length} defaultOpen={false}>
                        <div className="max-h-48 space-y-1.5 overflow-y-auto pr-1">
                            {relatedShares.map((s) => (
                                <button
                                    key={s.id}
                                    onClick={() => {
                                        revealLeftPanel()
                                        update({tag: s.tag_path, include: [], exclude: [], exact: [], panel: 'outgoing'})
                                    }}
                                    className="flex w-full items-center justify-between gap-2 text-xs hover:text-primary"
                                >
                                    <span className="min-w-0 truncate">→ @{s.recipient_username}:{s.recipient_instance}</span>
                                    <ShareStatusBadge status={s.status}/>
                                </button>
                            ))}
                        </div>
                    </Section>
                )}

                {/* EXIF section with inline editing */}
                <ExifInlineEditor picture={picture} exif={exif}/>

                <Section id="versions" title="Versions" count={picture.versions.length} defaultOpen={false}>
                    {picture.versions.length > 0 ? (
                        <div className="space-y-1.5">
                            {picture.versions.map((v) => (
                                <div key={v.id} className="flex items-baseline justify-between gap-2 text-sm">
                                    <span className="text-muted-foreground">v{v.version_number}</span>
                                    <span>{formatDateTime(v.created_at)}</span>
                                </div>
                            ))}
                        </div>
                    ) : (
                        <span className="text-xs text-muted-foreground">No previous versions.</span>
                    )}
                </Section>

                {/* Physical copies / content-dedup group (feature 11 §5.5). */}
                <CopiesSection pictureId={picture.id}/>
            </div>
        </div>
    )
}

// ── Panel ─────────────────────────────────────────────────────────────────────

export function SelectionPanel() {
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)

    const single = query === null && includeIds.length === 1 && excludeIds.length === 0
    const hasSelection = query !== null || includeIds.length > 0

    const [sp] = useSearchParams()
    const queryClient = useQueryClient()
    const {trash} = useTrashMutations()

    // Delete / ⌘+Backspace trashes the single selected picture directly, no confirmation. The
    // Lightbox handles the same shortcut itself while open, so skip here to avoid double-firing.
    useEffect(() => {
        if (!single || sp.has('view')) return
        const id = includeIds[0]
        const onKey = (e: KeyboardEvent) => {
            const t = e.target as HTMLElement | null
            if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
            if (e.key !== 'Delete' && !(e.metaKey && e.key === 'Backspace')) return
            const picture = queryClient.getQueryData<PictureDetail>(queryKeys.picture(id))
            if (picture?.deleted_at) return
            e.preventDefault()
            trash.mutate(id)
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [single, sp, includeIds, queryClient, trash])

    // The panel is now toggle-driven (it stays mounted with nothing selected so the grid never
    // shifts); render an unobtrusive placeholder rather than collapsing the layout.
    if (!hasSelection) {
        return (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center text-muted-foreground">
                <ImageIcon className="h-8 w-8 opacity-40"/>
                <p className="text-sm">No photo selected</p>
                <p className="text-xs text-muted-foreground/70">Select a photo to see its details here.</p>
            </div>
        )
    }

    return (
        <div className="h-full overflow-y-auto">
            {single ? <SinglePicture id={includeIds[0]}/> : <MultiSelectionPanel/>}
        </div>
    )
}
