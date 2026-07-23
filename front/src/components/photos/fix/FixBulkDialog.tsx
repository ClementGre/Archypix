// Bulk apply preview (feature 30 §8): one row per target showing the proposed value, **where it came
// from** (provenance), and — for GPS — the before/after references it was interpolated from, over a
// shared map. The user can edit any value (calendar / map picker), switch a date row's source, or
// toggle whole provenances on/off. Confirm loops the per-picture write, routing owned → write-through
// and received → local override / propose.

import {useMemo, useRef, useState} from 'react'
import {AlertTriangle, Check, Loader2, X} from 'lucide-react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger} from '@/components/ui/dropdown-menu'
import {GpsPickerPopover} from '@/components/photos/detail/GpsPickerPopover'
import {DateTimePickerPopover, formatNaive} from '@/components/photos/detail/DateTimePickerPopover'
import {OrientedContainImage} from '@/components/photos/OrientedImage'
import {MapView} from '@/components/common/MapView'
import {ReceivedModeToggle} from './ReceivedModeToggle'
import {type FixReceivedMode, type FixValue, useFixApply} from '@/hooks/useFixApply'
import {formatLatLng} from '@/lib/gpsInterpolation'
import {type BulkRow, type Provenance, PROVENANCE_LABEL} from '@/lib/fixBulk'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import type {FixMode} from '@/lib/types'

type RowStatus = 'idle' | 'saving' | 'done' | 'error' | 'grant_missing'

function valueLabel(field: FixMode, v: FixValue | null): string {
    if (!v) return 'skip'
    if (field === 'gps') return v.gps_lat == null || v.gps_lng == null ? 'skip' : formatLatLng(v.gps_lat, v.gps_lng)
    return v.captured_at ? formatNaive(v.captured_at) : 'skip'
}

/** A tiny reference thumbnail (a GPS before/after anchor) shown in a row. */
function MiniThumb({anchor, label}: { anchor: { thumbnail_url: string | null; orientation: number | null } | null; label: string }) {
    return (
        <div className="relative h-7 w-7 shrink-0 overflow-hidden rounded bg-checkerboard ring-1 ring-sky-400/50" title={label}>
            {anchor?.thumbnail_url && <OrientedContainImage src={anchor.thumbnail_url} alt="" orientation={anchor.orientation} className="h-7 w-7"/>}
        </div>
    )
}

export function FixBulkDialog({open, onOpenChange, field, title, initialRows, hasReceived, onApplied}: {
    open: boolean
    onOpenChange: (open: boolean) => void
    field: FixMode
    title: string
    initialRows: BulkRow[]
    hasReceived: boolean
    onApplied: () => void
}) {
    const {applyOne, invalidate} = useFixApply()
    const [rows, setRows] = useState<BulkRow[]>(initialRows)
    const [status, setStatus] = useState<Record<string, RowStatus>>({})
    const [receivedMode, setReceivedMode] = useState<FixReceivedMode>('local')
    const [running, setRunning] = useState(false)

    // Re-seed when the dialog transitions closed → open (render-time, not an effect).
    const prevOpen = useRef(open)
    if (open && !prevOpen.current) {
        setRows(initialRows)
        setStatus({})
        setRunning(false)
    }
    prevOpen.current = open

    const setRow = (id: string, patch: Partial<BulkRow>) => setRows((rs) => rs.map((r) => (r.id === id ? {...r, ...patch} : r)))

    // Provenance chips: toggle every row of a provenance in/out at once (§12 friendlier preview).
    const provenances = useMemo(() => {
        const seen = new Map<Provenance, number>()
        for (const r of rows) if (r.value && r.provenance) seen.set(r.provenance, (seen.get(r.provenance) ?? 0) + 1)
        return [...seen.entries()]
    }, [rows])
    const allIncluded = (p: Provenance) => rows.filter((r) => r.provenance === p && r.value).every((r) => r.include)
    const toggleProvenance = (p: Provenance) => {
        const inc = !allIncluded(p)
        setRows((rs) => rs.map((r) => (r.provenance === p && r.value ? {...r, include: inc} : r)))
    }

    const applicable = useMemo(() => rows.filter((r) => r.include && r.value), [rows])
    const mapPoints = useMemo(
        () => applicable.filter((r) => r.value?.gps_lat != null).map((r) => ({lat: r.value!.gps_lat!, lng: r.value!.gps_lng!, color: '#10b981'})),
        [applicable],
    )

    const run = async () => {
        setRunning(true)
        const next: Record<string, RowStatus> = {}
        for (const row of applicable) {
            setStatus((s) => ({...s, [row.id]: 'saving'}))
            try {
                await applyOne(row.id, row.owned, row.value!, receivedMode)
                next[row.id] = 'done'
            } catch (e) {
                next[row.id] = !row.owned && receivedMode === 'propose' ? 'grant_missing' : 'error'
                if (next[row.id] === 'error') toast.error(`Could not update ${row.filename ?? 'photo'}`, {description: apiErrorMessage(e)})
            }
            setStatus((s) => ({...s, [row.id]: next[row.id]}))
        }
        invalidate()
        setRunning(false)
        const ok = Object.values(next).filter((s) => s === 'done').length
        const failed = Object.values(next).filter((s) => s === 'error' || s === 'grant_missing').length
        if (ok) toast.success(`Updated ${ok} photo${ok > 1 ? 's' : ''}`)
        if (!failed) {
            onApplied()
            onOpenChange(false)
        }
    }

    return (
        <Dialog open={open} onOpenChange={(o) => !running && onOpenChange(o)}>
            <DialogContent className="max-w-lg">
                <DialogHeader>
                    <DialogTitle>{title}</DialogTitle>
                </DialogHeader>

                {/* Provenance filter: what each proposed value came from; click to include/exclude that source. */}
                {provenances.length > 0 && (
                    <div className="flex flex-wrap items-center gap-1.5">
                        <span className="text-xs text-muted-foreground">From:</span>
                        {provenances.map(([p, count]) => (
                            <button
                                key={p}
                                type="button"
                                onClick={() => toggleProvenance(p)}
                                className={cn(
                                    'flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] transition-colors',
                                    allIncluded(p) ? 'border-primary/60 bg-primary/10 text-primary' : 'border-border text-muted-foreground',
                                )}
                            >
                                <span
                                    className={cn('h-2.5 w-2.5 rounded-sm border', allIncluded(p) ? 'border-primary bg-primary' : 'border-border')}/>
                                {PROVENANCE_LABEL[p]} <span className="opacity-60">{count}</span>
                            </button>
                        ))}
                    </div>
                )}

                {hasReceived && (
                    <div className="flex items-center gap-2 text-xs">
                        <span className="text-muted-foreground">Received photos:</span>
                        <div className="flex-1"><ReceivedModeToggle value={receivedMode} onChange={setReceivedMode} allowPropose/></div>
                    </div>
                )}

                {/* GPS overview map of the proposed points. */}
                {field === 'gps' && mapPoints.length > 0 && (
                    <div className="overflow-hidden rounded-md border border-border">
                        <MapView mode="point" interactive={false} point={{lat: null, lng: null}} extraMarkers={mapPoints} className="h-40 w-full"/>
                    </div>
                )}

                <div className="max-h-[45vh] space-y-1 overflow-y-auto">
                    {rows.map((row) => {
                        const st = status[row.id] ?? 'idle'
                        const skipped = !row.include || !row.value
                        return (
                            <div key={row.id} className={cn('flex items-center gap-2 rounded-md px-1.5 py-1', skipped && 'opacity-50')}>
                                <button
                                    type="button"
                                    onClick={() => setRow(row.id, {include: !row.include})}
                                    className={cn('flex h-4 w-4 shrink-0 items-center justify-center rounded-sm border', row.include ? 'border-primary bg-primary text-primary-foreground' : 'border-border')}
                                    aria-label={row.include ? 'Exclude' : 'Include'}
                                >
                                    {row.include && <Check className="h-3 w-3"/>}
                                </button>
                                <div className="relative h-9 w-9 shrink-0 overflow-hidden rounded bg-checkerboard">
                                    {row.thumbnail_url &&
                                        <OrientedContainImage src={row.thumbnail_url} alt="" orientation={row.orientation} className="h-9 w-9"/>}
                                </div>

                                {/* GPS: the before/after references it was interpolated from. */}
                                {field === 'gps' && (row.before || row.after) && (
                                    <div className="flex items-center gap-0.5">
                                        <MiniThumb anchor={row.before ?? null} label="Before"/>
                                        <MiniThumb anchor={row.after ?? null} label="After"/>
                                    </div>
                                )}

                                <div className="min-w-0 flex-1">
                                    <p className="truncate text-xs" title={row.filename ?? ''}>
                                        {row.filename ?? 'Untitled'}
                                        {!row.owned && <span className="ml-1 text-[10px] text-muted-foreground">(received)</span>}
                                    </p>
                                    {/* Provenance: switchable for dates (choose a different source), a badge for GPS. */}
                                    {field === 'date' && row.dateSources && row.dateSources.length > 0 ? (
                                        <DropdownMenu>
                                            <DropdownMenuTrigger asChild>
                                                <button className="text-[10px] text-muted-foreground hover:text-primary">
                                                    {row.provenance ? PROVENANCE_LABEL[row.provenance] : 'source'} ▾
                                                </button>
                                            </DropdownMenuTrigger>
                                            <DropdownMenuContent align="start">
                                                {row.dateSources.map((s) => (
                                                    <DropdownMenuItem key={s.key} onSelect={() => setRow(row.id, {
                                                        value: {captured_at: s.value},
                                                        provenance: s.key
                                                    })}>
                                                        {PROVENANCE_LABEL[s.key]}: {formatNaive(s.value)}
                                                    </DropdownMenuItem>
                                                ))}
                                            </DropdownMenuContent>
                                        </DropdownMenu>
                                    ) : (
                                        row.provenance &&
                                        <span className="text-[10px] text-muted-foreground">{PROVENANCE_LABEL[row.provenance]}</span>
                                    )}
                                </div>

                                {/* Editable value. */}
                                {field === 'gps' ? (
                                    <GpsPickerPopover
                                        value={{
                                            lat: row.value?.gps_lat != null ? String(row.value.gps_lat) : '',
                                            lng: row.value?.gps_lng != null ? String(row.value.gps_lng) : '',
                                            alt: row.value?.gps_alt != null ? String(row.value.gps_alt) : '',
                                        }}
                                        onChange={(v) =>
                                            setRow(row.id, {
                                                value: v.lat && v.lng ? {
                                                    gps_lat: parseFloat(v.lat),
                                                    gps_lng: parseFloat(v.lng),
                                                    gps_alt: v.alt ? Math.round(parseFloat(v.alt)) : null
                                                } : null,
                                                provenance: 'manual',
                                            })
                                        }
                                    >
                                        <button
                                            className="max-w-[38%] shrink-0 truncate text-right text-xs text-primary hover:underline">{valueLabel('gps', row.value)}</button>
                                    </GpsPickerPopover>
                                ) : (
                                    <DateTimePickerPopover value={row.value?.captured_at ?? null} onChange={(v) => setRow(row.id, {
                                        value: v ? {captured_at: v} : null,
                                        provenance: 'manual'
                                    })}>
                                        <button
                                            className="max-w-[38%] shrink-0 truncate text-right text-xs text-primary hover:underline">{valueLabel('date', row.value)}</button>
                                    </DateTimePickerPopover>
                                )}

                                {st === 'saving' && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground"/>}
                                {st === 'done' && <Check className="h-3.5 w-3.5 shrink-0 text-emerald-500"/>}
                                {st === 'error' && <X className="h-3.5 w-3.5 shrink-0 text-destructive"/>}
                                {st === 'grant_missing' && <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-500"
                                                                          aria-label="This share does not allow proposing to the owner"/>}
                            </div>
                        )
                    })}
                </div>

                <DialogFooter>
                    <span className="mr-auto self-center text-xs text-muted-foreground">{applicable.length} of {rows.length} will change</span>
                    <Button variant="ghost" size="sm" disabled={running} onClick={() => onOpenChange(false)}>Cancel</Button>
                    <Button size="sm" disabled={running || applicable.length === 0} onClick={run}>
                        {running && <Loader2 className="mr-1 h-4 w-4 animate-spin"/>}
                        Apply to {applicable.length}
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
