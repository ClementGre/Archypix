import {useState} from 'react'
import {Link, useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {Copy, Download, ImageIcon, Loader2, LogIn} from 'lucide-react'
import {toast} from 'sonner'
import type {PublicPictureDetail} from '@/api/publicShares'
import {downloadPublicOriginal, getPublicPictureDetail, getPublicPictureUrl, publicAggregate, saveCopyFromPublic,} from '@/api/publicShares'
import {apiErrorMessage} from '@/api/client'
import {useAuthStore} from '@/stores/auth'
import {useSelectionStore} from '@/stores/selection'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {formatBytes, formatDateTime, isVideoMime} from '@/lib/utils'
import {Button} from '@/components/ui/button'
import {displayDimensions, OrientedContainImage} from '@/components/photos/OrientedImage'
import {FileTypeIcon} from '@/components/photos/FileTypeIcon'
import {PlayBadge} from '@/components/photos/PlayBadge'
import {Section} from '@/components/photos/detail/Section'
import {ReadOnlyRow} from '@/components/photos/detail/ExifInlineEditor'
import {usePublicShare} from '@/components/public/context'

/** Max height (px) of the panel preview — matches the app's SelectionPanel. */
const PREVIEW_MAX_HEIGHT = 208

export function PublicDetailPanel() {
    const ids = useSelectionStore((s) => s.includeIds)
    if (ids.length === 0) {
        return (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center text-muted-foreground">
                <ImageIcon className="h-8 w-8 opacity-40"/>
                <p className="text-sm">No photo selected</p>
                <p className="text-xs text-muted-foreground/70">Select a photo to see its details here.</p>
            </div>
        )
    }
    if (ids.length === 1) {
        return <SingleDetail id={ids[0]}/>
    }
    return <BatchDetail ids={ids}/>
}

// ── Single ────────────────────────────────────────────────────────────────────

function SingleDetail({id}: { id: string }) {
    const {backendUrl, token, session, meta, ownerUsername, globalDomain} = usePublicShare()
    const user = useAuthStore((s) => s.user)
    const instance = useAuthStore((s) => s.instance)
    const isOwner = !!user && user.username === ownerUsername && (instance || GLOBAL_DOMAIN) === globalDomain
    const [, setSp] = useSearchParams()
    const [busy, setBusy] = useState(false)
    const canOriginals = meta.permissions.allow_originals

    const detailQ = useQuery({
        queryKey: ['publicDetail', backendUrl, token, id],
        queryFn: () => getPublicPictureDetail(backendUrl, token, id, session),
        retry: false,
    })
    const previewQ = useQuery({
        queryKey: ['publicUrl', token, id, 'medium'],
        queryFn: () => getPublicPictureUrl(backendUrl, token, id, 'medium', session),
        retry: false,
    })

    if (detailQ.isPending) return <PanelLoader/>
    if (detailQ.isError || !detailQ.data) return <Empty>Could not load this photo.</Empty>
    const d = detailQ.data
    const previewUrl = previewQ.data ?? undefined
    const isVideo = isVideoMime(d.mime_type)
    const dims = displayDimensions(d.width, d.height, d.orientation)

    const openLightbox = () =>
        setSp((prev) => {
            const next = new URLSearchParams(prev)
            next.set('view', id)
            return next
        })

    const download = async () => {
        try {
            await downloadPublicOriginal(backendUrl, token, id, d.filename, session)
        } catch {
            toast.error('Download is not available for this album.')
        }
    }
    const saveCopy = async () => {
        setBusy(true)
        try {
            await saveCopyFromPublic({owner_username: ownerUsername, owner_instance: globalDomain, token, picture_id: id})
            toast.success('Saved a copy to your library.')
        } catch (e) {
            toast.error(apiErrorMessage(e))
        } finally {
            setBusy(false)
        }
    }

    return (
        <div>
            {/* Preview — borderless, full width, top-aligned in its reserved space; click opens the lightbox. */}
            <div
                className="group relative w-full cursor-pointer overflow-hidden bg-muted"
                onClick={openLightbox}
                title="Open full screen"
            >
                {isVideo && previewUrl ? (
                    <div className="relative flex items-center justify-center bg-black">
                        <img src={previewUrl} alt={d.filename ?? ''} className="max-h-52 w-full object-contain"/>
                        <PlayBadge hover/>
                    </div>
                ) : previewUrl ? (
                    <OrientedContainImage
                        src={previewUrl}
                        blurhash={d.blurhash}
                        alt={d.filename ?? ''}
                        orientation={d.orientation}
                        width={d.width}
                        height={d.height}
                        maxHeight={PREVIEW_MAX_HEIGHT}
                    />
                ) : (
                    <div className="flex h-40 items-center justify-center text-muted-foreground">
                        <FileTypeIcon mime={d.mime_type} filename={d.filename} className="h-12 w-12 opacity-70"/>
                    </div>
                )}
            </div>

            {/* Info row — filename + size/dims/mime, with the download action (matches the app). */}
            <div className="flex items-start justify-between gap-1 px-3 pt-2 pb-1">
                <div className="min-w-0">
                    <p className="truncate text-sm font-medium" title={d.filename ?? undefined}>
                        {d.filename ?? 'Untitled'}
                    </p>
                    <p className="text-xs text-muted-foreground">
                        {[
                            formatBytes(d.file_size),
                            dims.width && dims.height ? `${dims.width} × ${dims.height}` : null,
                            d.mime_type,
                        ]
                            .filter(Boolean)
                            .join(' · ')}
                    </p>
                </div>
                {canOriginals && (
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                        title="Download original"
                        onClick={download}
                    >
                        <Download className="h-4 w-4"/>
                    </Button>
                )}
            </div>

            <div className="px-3 pb-2 text-xs text-muted-foreground">Added {formatDateTime(d.ingested_at)}</div>

            <CreatorLine creator={d.creator}/>

            {/* Per-picture action — save a copy (or a sign-in prompt). Convert-to-a-share is a whole-album
                action, so it lives only in the top bar. Hidden from the album owner. */}
            {canOriginals && !isOwner && (
                <div className="flex flex-wrap gap-2 px-3 pb-3">
                    {user ? (
                        <Button size="sm" variant="secondary" disabled={busy} onClick={saveCopy}>
                            <Copy className="mr-1.5 h-4 w-4"/> Save a copy
                        </Button>
                    ) : (
                        <Button size="sm" variant="outline" asChild>
                            <Link to="/login">
                                <LogIn className="mr-1.5 h-4 w-4"/> Sign in to save a copy
                            </Link>
                        </Button>
                    )}
                </div>
            )}

            {!meta.view_only && (
                <div className="px-3">
                    <PublicExifSection d={d}/>
                </div>
            )}
        </div>
    )
}

function CreatorLine({creator}: { creator: string }) {
    // `#name` contributions show just the name (the "Created by" prefix already carries the credit);
    // an `@user:domain` identity or a plain string render as-is.
    const isContributor = creator.startsWith('#')
    const label = isContributor ? creator.slice(1) : creator
    return (
        <div className="px-3 pb-2 text-xs">
            <span className="text-muted-foreground">Created by </span>
            <span className="font-medium">{label}</span>
            {isContributor && (
                <span className="ml-1.5 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">public share</span>
            )}
        </div>
    )
}

const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null)
const numStr = (v: unknown): string | null => (typeof v === 'number' && isFinite(v) ? String(v) : null)

/** Read-only EXIF section — reuses the app's `Section` + `ReadOnlyRow` (FieldLabel chips). */
function PublicExifSection({d}: { d: PublicPictureDetail }) {
    const ex = (d.exif_data ?? {}) as Record<string, unknown>
    const focal = numStr(ex.focal_length_mm)
    const fnum = numStr(ex.f_number)
    const iso = numStr(ex.iso_speed)
    const expNum = numStr(ex.exposure_time_num)
    const expDen = numStr(ex.exposure_time_den)
    const rows: Array<[string, string | null]> = [
        ['Captured at', d.captured_at ? formatDateTime(d.captured_at) : null],
        [
            'GPS',
            d.gps_lat != null && d.gps_lng != null
                ? `${d.gps_lat.toFixed(5)}, ${d.gps_lng.toFixed(5)}${d.gps_alt != null ? ` · ${d.gps_alt} m` : ''}`
                : null,
        ],
        ['Camera brand', str(ex.camera_brand)],
        ['Camera model', str(ex.camera_model)],
        ['Focal length', focal ? `${focal} mm` : null],
        ['Aperture', fnum ? `f/${fnum}` : null],
        ['ISO', iso ? `ISO ${iso}` : null],
        ['Exposure', expNum && expDen ? `${expNum}/${expDen} s` : null],
    ]
    const present = rows.filter(([, v]) => v) as Array<[string, string]>
    if (present.length === 0) return null
    // Distinct id + open by default (the public panel expands EXIF; doesn't share the app's collapse state).
    return (
        <Section id="public-exif" title="EXIF" defaultOpen>
            <div className="space-y-0.5">
                {present.map(([label, value]) => (
                    <ReadOnlyRow key={label} label={label} value={value}/>
                ))}
            </div>
        </Section>
    )
}

// ── Batch (aggregate over the selection) ────────────────────────────────────────

interface ExifAgg {
    type: 'distinct' | 'numeric' | 'date'
    common?: string | null
    distinct?: { value: string; count: number }[]
    min?: number | string | null
    max?: number | string | null
}

function BatchDetail({ids}: { ids: string[] }) {
    const {backendUrl, token, session, meta} = usePublicShare()
    const sections = meta.view_only ? ['summary'] : ['summary', 'exif']
    const aggQ = useQuery({
        queryKey: ['publicAggregate', backendUrl, token, [...ids].sort().join(','), sections.join(',')],
        queryFn: () => publicAggregate(backendUrl, token, ids, sections, session),
        retry: false,
    })

    return (
        <div className="p-3 text-sm">
            <div className="px-1 pb-2 font-medium">{ids.length} photos selected</div>
            {aggQ.isPending ? (
                <PanelLoader/>
            ) : aggQ.isError || !aggQ.data ? (
                <Empty>Could not aggregate the selection.</Empty>
            ) : (
                <>
                    <Section id="batch-info" title="Info">
                        <div className="space-y-0.5">
                            <ReadOnlyRow label="In selection" value={String(aggQ.data.count ?? ids.length)}/>
                            <ReadOnlyRow label="Total size" value={formatBytes(aggQ.data.total_file_size ?? null)}/>
                        </div>
                    </Section>
                    {!meta.view_only && aggQ.data.exif ? (
                        <BatchExif exif={aggQ.data.exif as Record<string, ExifAgg>}/>
                    ) : null}
                </>
            )}
        </div>
    )
}

function BatchExif({exif}: { exif: Record<string, ExifAgg> }) {
    const fields: [string, string][] = [
        ['captured_at', 'Captured at'],
        ['camera_brand', 'Camera brand'],
        ['camera_model', 'Camera model'],
        ['iso_speed', 'ISO'],
        ['f_number', 'Aperture'],
        ['focal_length_mm', 'Focal length'],
    ]
    const render = (a: ExifAgg | undefined): string | null => {
        if (!a) return null
        if (a.type === 'distinct') {
            if (a.common) return a.common
            const n = a.distinct?.length ?? 0
            return n > 0 ? `${n} values` : null
        }
        if (a.min != null && a.max != null) {
            return a.min === a.max ? String(a.min) : `${a.min} – ${a.max}`
        }
        return null
    }
    const rows = fields
        .map(([key, label]) => [label, render(exif[key])] as const)
        .filter(([, v]) => v) as Array<[string, string]>
    if (rows.length === 0) return null
    return (
        <Section id="batch-exif" title="EXIF summary" defaultOpen={false}>
            <div className="space-y-0.5">
                {rows.map(([label, value]) => (
                    <ReadOnlyRow key={label} label={label} value={value}/>
                ))}
            </div>
        </Section>
    )
}

function Empty({children}: { children: React.ReactNode }) {
    return <div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted-foreground">{children}</div>
}

function PanelLoader() {
    return (
        <div className="flex justify-center p-8">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground"/>
        </div>
    )
}
