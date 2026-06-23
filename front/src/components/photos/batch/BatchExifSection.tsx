import {type ReactNode, useMemo, useRef, useState} from 'react'
import {toast} from 'sonner'
import {Info, Loader2, RotateCcw, Save} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Checkbox} from '@/components/ui/checkbox'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Section} from '@/components/photos/detail/Section'
import {FieldLabel} from '@/components/photos/detail/FieldLabel'
import {DateTimePickerPopover, formatNaive} from '@/components/photos/detail/DateTimePickerPopover'
import {GpsPickerPopover} from '@/components/photos/detail/GpsPickerPopover'
import {MapView} from '@/components/common/MapView'
import {BatchConfirmDialog} from './BatchConfirmDialog'
import {batchEditExif} from '@/api/pictures'
import {useBatchMutations} from '@/hooks/useBatch'
import {cn, formatBytes} from '@/lib/utils'
import type {BatchDryRun, BatchExifMode, ExifField, ExifOverrides, FieldAggregate, PictureSelection} from '@/lib/types'

// ── value formatting ──────────────────────────────────────────────────────────

function formatScalar(field: string, value: unknown): string {
    if (value == null || value === '') return '—'
    switch (field) {
        case 'file_size':
            return formatBytes(Number(value))
        case 'f_number':
            return `f/${value}`
        case 'focal_length_mm':
            return `${value} mm`
        case 'iso_speed':
            return `ISO ${value}`
        case 'gps_alt':
            return `${value} m`
        case 'width':
        case 'height':
            return `${value} px`
        default:
            return String(value)
    }
}

const r2 = (n: number | null) => (n == null ? null : Math.round(n * 100) / 100)

function shortDate(iso: string | null): string {
    if (!iso) return '—'
    const hasTz = /[zZ]$|[+-]\d{2}:?\d{2}$/.test(iso)
    const d = new Date(!hasTz && iso.includes('T') ? `${iso}Z` : iso)
    return Number.isNaN(d.getTime()) ? '—' : d.toLocaleDateString(undefined, {year: 'numeric', month: 'short', day: 'numeric'})
}

/** The single primary value to show, or "Mixed" when the field varies. */
function primary(field: string, agg: FieldAggregate | undefined): { text: string; mixed: boolean } {
    if (!agg) return {text: '—', mixed: false}
    if (agg.type === 'distinct') {
        if (agg.common != null) return {text: formatScalar(field, agg.common), mixed: false}
        if (agg.distinct.length || agg.distinct_overflow) return {text: 'Mixed', mixed: true}
        return {text: '—', mixed: false}
    }
    if (agg.type === 'numeric') {
        if (agg.min == null && agg.max == null) return {text: '—', mixed: false}
        if (agg.min === agg.max) return {text: formatScalar(field, r2(agg.min)), mixed: false}
        return {text: 'Mixed', mixed: true}
    }
    if (agg.type === 'date') {
        if (!agg.min && !agg.max) return {text: '—', mixed: false}
        if (agg.min === agg.max) return {text: shortDate(agg.min), mixed: false}
        return {text: 'Mixed', mixed: true}
    }
    return {text: '—', mixed: false}
}

/** "n/total set" when some are missing, else null. Always rendered first in a stats row. */
function setText(agg: FieldAggregate | undefined, total: number): string | null {
    return agg && agg.null_count > 0 ? `${total - agg.null_count}/${total} set` : null
}

/**
 * The small stats sub-row: the set count first, then the range / avg / distinct values. Rendered on a
 * single truncated line, so a long string-value list naturally shows the first few with an ellipsis.
 */
function statsText(field: string, agg: FieldAggregate | undefined, total: number): string | null {
    if (!agg) return null
    const parts: string[] = []
    const set = setText(agg, total)
    if (set) parts.push(set)
    if (agg.type === 'numeric') {
        if (agg.min != null && agg.min !== agg.max) {
            parts.push(`${formatScalar(field, r2(agg.min))} – ${formatScalar(field, r2(agg.max))}`)
            if (agg.avg != null) parts.push(`avg ${formatScalar(field, r2(agg.avg))}`)
        }
    } else if (agg.type === 'date') {
        if (agg.min && agg.min !== agg.max) parts.push(`${shortDate(agg.min)} – ${shortDate(agg.max)}`)
    } else if (agg.type === 'distinct') {
        // Mixed strings: list the distinct values; the single-line ellipsis trims the overflow.
        if (agg.common == null && agg.distinct.length) {
            const list = agg.distinct.map((d) => formatScalar(field, d.value)).join(', ')
            parts.push(agg.distinct_overflow > 0 ? `${list} +${agg.distinct_overflow}` : list)
        }
    }
    return parts.length ? parts.join(' · ') : null
}

// ── small building blocks ───────────────────────────────────────────────────────

function DirtyDot({dirty}: { dirty: boolean }) {
    return <div className="flex w-3 shrink-0 items-center justify-center">{dirty && <div className="h-1.5 w-1.5 rounded-full bg-primary"/>}</div>
}

function ResetSlot({dirty, onReset}: { dirty: boolean; onReset: () => void }) {
    return (
        <div className="flex w-4 shrink-0 items-center justify-center">
            {dirty && (
                <button onClick={onReset} title="Reset"
                        className="text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100">
                    <RotateCcw className="h-3 w-3"/>
                </button>
            )}
        </div>
    )
}

/** (i) popover listing the distinct values of a string/enum field with their counts. */
function DistinctPopover({label, field, agg}: { label: string; field: string; agg: Extract<FieldAggregate, { type: 'distinct' }> }) {
    return (
        <Popover>
            <PopoverTrigger asChild>
                <button className="text-muted-foreground hover:text-foreground" aria-label={`Show ${label} values`}>
                    <Info className="h-3.5 w-3.5"/>
                </button>
            </PopoverTrigger>
            <PopoverContent className="w-56 p-2" align="end">
                <p className="mb-1 text-xs font-medium">{label}</p>
                <div className="max-h-60 space-y-0.5 overflow-y-auto">
                    {agg.distinct.map((d, i) => (
                        <div key={i} className="flex items-baseline justify-between gap-2 text-xs">
                            <span className="min-w-0 truncate">{formatScalar(field, d.value)}</span>
                            <span className="shrink-0 tabular-nums text-muted-foreground">{d.count}</span>
                        </div>
                    ))}
                </div>
                {agg.distinct_overflow > 0 && <p className="mt-1 text-[11px] text-muted-foreground">+{agg.distinct_overflow} more values</p>}
            </PopoverContent>
        </Popover>
    )
}

/** A field row: green label + value + optional dirty/reset, with a stats sub-row underneath. */
function Row({label, dirty, onReset, stats, children}: {
    label: string;
    dirty?: boolean;
    onReset?: () => void;
    stats: string | null;
    children: ReactNode
}) {
    return (
        <div className="group">
            <div className="flex min-h-[1.4rem] items-center gap-1.5">
                <DirtyDot dirty={!!dirty}/>
                <FieldLabel>{label}</FieldLabel>
                <div className="flex min-w-0 flex-1 items-center justify-end gap-1">{children}</div>
                {onReset ? <ResetSlot dirty={!!dirty} onReset={onReset}/> : <div className="w-4 shrink-0"/>}
            </div>
            {stats && (
                <div className="flex items-center gap-1 pb-0.5">
                    <div className="w-3 shrink-0"/>
                    <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground" title={stats}>{stats}</span>
                    <div className="w-4 shrink-0"/>
                </div>
            )}
        </div>
    )
}

/** Click-to-edit inline scalar input; when not touched it shows the aggregate value. */
function ScalarEdit({field, agg, draftVal, onChange, label, type = 'text', step}: {
    field: string
    agg: FieldAggregate | undefined
    draftVal: string | undefined
    onChange: (v: string) => void
    label: string
    type?: 'text' | 'number'
    step?: string | number
}) {
    const [editing, setEditing] = useState(false)
    const [input, setInput] = useState('')
    const ref = useRef<HTMLInputElement>(null)
    const touched = draftVal !== undefined
    const prim = primary(field, agg)

    const start = () => {
        setInput(draftVal ?? '')
        setEditing(true)
        setTimeout(() => ref.current?.select(), 0)
    }
    const commit = () => {
        setEditing(false)
        onChange(input)
    }

    const display = touched ? (draftVal!.trim() === '' ? 'cleared' : formatScalar(field, draftVal)) : prim.text

    if (editing) {
        return (
            <input
                ref={ref}
                type={type}
                value={input}
                step={step}
                onChange={(e) => setInput(e.target.value)}
                onBlur={commit}
                onKeyDown={(e) => {
                    if (e.key === 'Enter') commit()
                    if (e.key === 'Escape') setEditing(false)
                }}
                className="w-full rounded border border-input bg-background px-1.5 py-0.5 text-right text-xs focus:outline-none focus:ring-1 focus:ring-ring"
            />
        )
    }
    return (
        <>
            {!touched && agg?.type === 'distinct' && prim.mixed && <DistinctPopover label={label} field={field} agg={agg}/>}
            <button
                onClick={start}
                className={cn(
                    'min-w-0 truncate rounded px-1 text-right text-xs transition-colors hover:bg-muted',
                    (display === '—' || display === 'cleared') && 'text-muted-foreground',
                    prim.mixed && !touched && 'text-amber-400',
                )}
            >
                {display}
            </button>
        </>
    )
}

// ── read-only row (used by the Metadata section) ────────────────────────────────

function ReadRow({field, label, agg, total}: { field: string; label: string; agg: FieldAggregate | undefined; total: number }) {
    const prim = primary(field, agg)
    return (
        <Row label={label} stats={statsText(field, agg, total)}>
            {agg?.type === 'distinct' && prim.mixed && <DistinctPopover label={label} field={field} agg={agg}/>}
            <span className={cn('truncate text-xs', prim.mixed ? 'text-amber-400' : prim.text === '—' && 'text-muted-foreground')}>{prim.text}</span>
        </Row>
    )
}

// ── Metadata (read-only) section ────────────────────────────────────────────────

/** Read-only file/metadata aggregates (the fields that aren't part of the single-picture EXIF editor). */
export function BatchMetadataSection({exif, total, open, onOpenChange}: {
    exif: Record<string, FieldAggregate> | undefined
    total: number
    open: boolean
    onOpenChange: (open: boolean) => void
}) {
    return (
        <Section id="multi-meta" title="Info" open={open} onOpenChange={onOpenChange}>
            {!exif ? (
                <div className="h-12 animate-pulse rounded bg-muted"/>
            ) : total === 0 ? (
                <span className="text-xs text-muted-foreground">No info.</span>
            ) : (
                <div className="space-y-0.5">
                    <ReadRow field="file_size" label="File size" agg={exif.file_size} total={total}/>
                    <ReadRow field="width" label="Width" agg={exif.width} total={total}/>
                    <ReadRow field="height" label="Height" agg={exif.height} total={total}/>
                    <ReadRow field="mime_type" label="Type" agg={exif.mime_type} total={total}/>
                    <ReadRow field="ingested_at" label="Added" agg={exif.ingested_at} total={total}/>
                    <ReadRow field="updated_at" label="Edited" agg={exif.updated_at} total={total}/>
                </div>
            )}
        </Section>
    )
}

// ── EXIF (editable) section ─────────────────────────────────────────────────────

// Editable scalar fields, in the SAME order as the single-picture EXIF editor.
const SCALARS: Array<{ field: ExifField; label: string; type?: 'text' | 'number'; step?: string | number }> = [
    {field: 'camera_brand', label: 'Camera brand'},
    {field: 'camera_model', label: 'Camera model'},
    {field: 'focal_length_mm', label: 'Focal length', type: 'number', step: 'any'},
    {field: 'f_number', label: 'Aperture', type: 'number', step: 'any'},
    {field: 'iso_speed', label: 'ISO', type: 'number', step: 1},
]

interface GpsStr {
    lat: string
    lng: string
    alt: string
}

export function BatchExifSection({exif, total, selection, hasReceived, open, onOpenChange}: {
    exif: Record<string, FieldAggregate> | undefined
    total: number
    selection: PictureSelection
    hasReceived: boolean
    open: boolean
    onOpenChange: (open: boolean) => void
}) {
    const {exif: exifMutation} = useBatchMutations()

    // Draft: a field key present in `draft` (or a defined `gpsDraft`) means it's being changed;
    // an empty value ⇒ clear, a non-empty value ⇒ set.
    const [draft, setDraft] = useState<Record<string, string>>({})
    const [gpsDraft, setGpsDraft] = useState<GpsStr | undefined>(undefined)
    const [mode, setMode] = useState<BatchExifMode>('local')

    const setField = (f: string, v: string) => setDraft((d) => ({...d, [f]: v}))
    const resetField = (...fs: string[]) =>
        setDraft((d) => {
            const n = {...d}
            for (const f of fs) delete n[f]
            return n
        })
    const resetAll = () => {
        setDraft({})
        setGpsDraft(undefined)
    }

    const {set, clear, dirty} = useMemo(() => {
        const set: Partial<ExifOverrides> = {}
        const clear: ExifField[] = []
        const num = (s: string) => (s.trim() === '' || isNaN(Number(s)) ? null : Number(s))
        for (const [f, v] of Object.entries(draft)) {
            if (f === 'captured_at') {
                v ? (set.captured_at = v) : clear.push('captured_at')
            } else if (f === 'camera_brand' || f === 'camera_model') {
                v.trim() ? ((set as Record<string, unknown>)[f] = v.trim()) : clear.push(f as ExifField)
            } else {
                const n = num(v)
                n != null ? ((set as Record<string, unknown>)[f] = n) : clear.push(f as ExifField)
            }
        }
        if (gpsDraft) {
            const lat = num(gpsDraft.lat)
            const lng = num(gpsDraft.lng)
            if (lat != null && lng != null) {
                set.gps_lat = lat
                set.gps_lng = lng
                const alt = num(gpsDraft.alt)
                if (alt != null) set.gps_alt = alt
            } else {
                clear.push('gps_lat', 'gps_lng', 'gps_alt')
            }
        }
        return {set, clear, dirty: Object.keys(draft).length > 0 || gpsDraft !== undefined}
    }, [draft, gpsDraft])

    const apply = () => {
        exifMutation.mutate(
            {selection, set, clear, mode},
            {
                onSuccess: (res) => {
                    if ('affected' in res) toast.success(`Edited EXIF on ${res.affected} ${res.affected === 1 ? 'photo' : 'photos'}`)
                    resetAll()
                },
            },
        )
    }

    // GPS row value + map
    const gpsAgg = exif?.gps?.type === 'gps' ? (exif.gps as Extract<FieldAggregate, { type: 'gps' }>) : null
    const gpsDirty = gpsDraft !== undefined
    const gpsDisplay = gpsDirty
        ? gpsDraft!.lat && gpsDraft!.lng
            ? `${gpsDraft!.lat}, ${gpsDraft!.lng}`
            : 'cleared'
        : gpsAgg?.centroid
            ? `~ ${gpsAgg.centroid.lat.toFixed(3)}, ${gpsAgg.centroid.lng.toFixed(3)}`
            : '—'
    const gpsStats = gpsAgg ? `${total - gpsAgg.null_count}/${total} have GPS` : null

    const header = dirty ? (
        <div className="flex items-center gap-1">
            <Badge variant="outline" className="h-5 border-primary px-1.5 text-[10px] text-primary">modified</Badge>
            <BatchConfirmDialog
                trigger={
                    <Button variant="ghost" size="icon" className="h-6 w-6 text-primary" title="Apply EXIF changes" disabled={exifMutation.isPending}>
                        {exifMutation.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <Save className="h-3.5 w-3.5"/>}
                    </Button>
                }
                title="Apply EXIF changes?"
                description="Owned pictures are written through (the file reconciles in the background); received pictures get a local override."
                confirmLabel="Apply"
                dryRun={() => batchEditExif({selection, set, clear, mode, dry_run: true})}
                dryRunKey={mode}
                renderResult={(r: BatchDryRun) => {
                    const bits: string[] = []
                    if (r.edited) bits.push(`${r.edited} edited`)
                    if (r.suggested) bits.push(`${r.suggested} suggested to owner`)
                    if (r.local_override) bits.push(`${r.local_override} local override`)
                    if (r.unsupported) bits.push(`${r.unsupported} unsupported`)
                    return (
                        <span>
                            <span className="font-medium tabular-nums">{r.affected}</span> affected
                            {bits.length > 0 && <span className="text-muted-foreground"> · {bits.join(', ')}</span>}
                        </span>
                    )
                }}
                onConfirm={apply}
            >
                {hasReceived && (
                    <label className="flex cursor-pointer items-start gap-2 rounded-md border border-border p-2 text-sm">
                        <Checkbox checked={mode === 'suggest'} onCheckedChange={(c) => setMode(c ? 'suggest' : 'local')} className="mt-0.5"/>
                        <span>
                            <span className="font-medium">Suggest to owner where allowed</span>
                            <span className="block text-xs text-muted-foreground">
                                Received pictures whose share grants editing are proposed to the owner; others get a private local override.
                            </span>
                        </span>
                    </label>
                )}
            </BatchConfirmDialog>
        </div>
    ) : undefined

    return (
        <Section id="multi-exif" title="EXIF" open={open} onOpenChange={onOpenChange} action={header}>
            {!exif ? (
                <div className="h-24 animate-pulse rounded bg-muted"/>
            ) : total === 0 ? (
                <span className="text-xs text-muted-foreground">No EXIF.</span>
            ) : (
                <div className="space-y-0.5">
                    {/* Captured */}
                    <Row label="Captured" dirty={draft.captured_at !== undefined} onReset={() => resetField('captured_at')}
                         stats={statsText('captured_at', exif.captured_at, total)}>
                        <DateTimePickerPopover
                            value={draft.captured_at !== undefined ? (draft.captured_at || null) : null}
                            onChange={(v) => setField('captured_at', v ?? '')}
                        >
                            {(() => {
                                const touched = draft.captured_at !== undefined
                                const prim = primary('captured_at', exif.captured_at)
                                const display = touched ? (draft.captured_at ? formatNaive(draft.captured_at) : 'cleared') : prim.text
                                return (
                                    <button
                                        className={cn(
                                            'truncate rounded px-1 text-right text-xs transition-colors hover:bg-muted',
                                            !touched && prim.mixed && 'text-amber-400',
                                            (display === '—' || display === 'cleared') && 'text-muted-foreground',
                                        )}
                                    >
                                        {display}
                                    </button>
                                )
                            })()}
                        </DateTimePickerPopover>
                    </Row>

                    {/* GPS + read-only map */}
                    <Row label="GPS" dirty={gpsDirty} onReset={() => setGpsDraft(undefined)} stats={gpsStats}>
                        <GpsPickerPopover value={gpsDraft ?? {lat: '', lng: '', alt: ''}} onChange={setGpsDraft}>
                            <button
                                className={cn('truncate rounded px-1 text-right text-xs transition-colors hover:bg-muted', (gpsDisplay === '—' || gpsDisplay === 'cleared') && 'text-muted-foreground')}>
                                {gpsDisplay}
                            </button>
                        </GpsPickerPopover>
                    </Row>
                    {gpsAgg?.bbox && (
                        <div className="mt-1 overflow-hidden rounded-md border border-border">
                            <MapView
                                mode="bbox"
                                interactive={false}
                                expandable={false}
                                bbox={{
                                    latMin: gpsAgg.bbox.lat_min,
                                    latMax: gpsAgg.bbox.lat_max,
                                    lonMin: gpsAgg.bbox.lng_min,
                                    lonMax: gpsAgg.bbox.lng_max
                                }}
                                onBbox={() => {
                                }}
                                className="h-40 w-full"
                            />
                        </div>
                    )}

                    {/* Camera scalars */}
                    {SCALARS.map((s) => (
                        <Row key={s.field} label={s.label} dirty={draft[s.field] !== undefined} onReset={() => resetField(s.field)}
                             stats={statsText(s.field, exif[s.field], total)}>
                            <ScalarEdit field={s.field} label={s.label} agg={exif[s.field]} draftVal={draft[s.field]}
                                        onChange={(v) => setField(s.field, v)} type={s.type} step={s.step}/>
                        </Row>
                    ))}

                    {/* Exposure (numerator / denominator merged into one rational field) */}
                    <ExposureRow
                        numAgg={exif.exposure_time_num}
                        denAgg={exif.exposure_time_den}
                        numDraft={draft.exposure_time_num}
                        denDraft={draft.exposure_time_den}
                        total={total}
                        onChangeNum={(v) => setField('exposure_time_num', v)}
                        onChangeDen={(v) => setField('exposure_time_den', v)}
                        onReset={() => resetField('exposure_time_num', 'exposure_time_den')}
                    />
                </div>
            )}
        </Section>
    )
}

/** Common single value of a numeric aggregate (min===max & not null), else null. */
function commonNumeric(agg: FieldAggregate | undefined): number | null {
    if (agg?.type === 'numeric' && agg.min != null && agg.min === agg.max) return agg.min
    return null
}

/** Exposure row: edits numerator/denominator together, displays `n/d s` or Mixed. */
function ExposureRow({numAgg, denAgg, numDraft, denDraft, total, onChangeNum, onChangeDen, onReset}: {
    numAgg: FieldAggregate | undefined
    denAgg: FieldAggregate | undefined
    numDraft: string | undefined
    denDraft: string | undefined
    total: number
    onChangeNum: (v: string) => void
    onChangeDen: (v: string) => void
    onReset: () => void
}) {
    const [editing, setEditing] = useState(false)
    const touched = numDraft !== undefined || denDraft !== undefined

    const commonNum = commonNumeric(numAgg)
    const commonDen = commonNumeric(denAgg)
    const mixed =
        (numAgg?.type === 'numeric' && numAgg.min !== numAgg.max) ||
        (denAgg?.type === 'numeric' && denAgg.min !== denAgg.max)

    let display: string
    if (touched) {
        display = numDraft || denDraft ? `${numDraft || '?'}/${denDraft || '?'} s` : 'cleared'
    } else if (commonNum != null && commonDen != null) {
        display = `${commonNum}/${commonDen} s`
    } else if (mixed) {
        display = 'Mixed'
    } else {
        display = '—'
    }

    // Concise stats: set count first, then a num / den range when they vary.
    const expStats = (() => {
        const parts: string[] = []
        const set = setText(numAgg, total)
        if (set) parts.push(set)
        if (numAgg?.type === 'numeric' && numAgg.min != null && numAgg.min !== numAgg.max) parts.push(`num ${r2(numAgg.min)}–${r2(numAgg.max)}`)
        if (denAgg?.type === 'numeric' && denAgg.min != null && denAgg.min !== denAgg.max) parts.push(`den ${r2(denAgg.min)}–${r2(denAgg.max)}`)
        return parts.length ? parts.join(' · ') : null
    })()

    const handleBlur = (e: React.FocusEvent<HTMLDivElement>) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setEditing(false)
    }

    return (
        <Row label="Exposure" dirty={touched} onReset={onReset} stats={expStats}>
            {editing ? (
                <div className="flex items-center gap-1" onBlur={handleBlur}>
                    <input
                        type="number"
                        step={1}
                        autoFocus
                        placeholder="1"
                        value={numDraft ?? ''}
                        onChange={(e) => onChangeNum(e.target.value)}
                        className="w-12 rounded border border-input bg-background px-1 py-0.5 text-right text-xs focus:outline-none focus:ring-1 focus:ring-ring"
                    />
                    <span className="text-xs text-muted-foreground">/</span>
                    <input
                        type="number"
                        step={1}
                        placeholder="200"
                        value={denDraft ?? ''}
                        onChange={(e) => onChangeDen(e.target.value)}
                        className="w-14 rounded border border-input bg-background px-1 py-0.5 text-right text-xs focus:outline-none focus:ring-1 focus:ring-ring"
                    />
                    <span className="text-xs text-muted-foreground">s</span>
                </div>
            ) : (
                <button
                    onClick={() => setEditing(true)}
                    className={cn(
                        'truncate rounded px-1 text-right text-xs transition-colors hover:bg-muted',
                        (display === '—' || display === 'cleared') && 'text-muted-foreground',
                        mixed && !touched && 'text-amber-400',
                    )}
                >
                    {display}
                </button>
            )}
        </Row>
    )
}
