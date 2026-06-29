import {useMemo, useState} from 'react'
import {AlertTriangle, ChevronDown, ChevronRight, GripVertical, Plus, Save, Scissors, Trash2, Undo2} from 'lucide-react'
import {toast} from 'sonner'
import type {DragEndEvent} from '@dnd-kit/core'
import {closestCenter, DndContext, PointerSensor, useSensor, useSensors} from '@dnd-kit/core'
import {SortableContext, useSortable, verticalListSortingStrategy} from '@dnd-kit/sortable'
import {CSS} from '@dnd-kit/utilities'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Switch} from '@/components/ui/switch'
import {Checkbox} from '@/components/ui/checkbox'
import {NumberInput} from '@/components/ui/number-input'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {DateRangePicker} from '@/components/common/DateRangePicker'
import {TagPicker} from '@/components/tags/TagPicker'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {
    displayDepths,
    lintConfig,
    NAMED_PLACEHOLDERS,
    newBand,
    parseDate,
    placeholderDefaultsToName,
    previewBuckets,
    renderBandPath,
    templatePlaceholders,
} from '@/lib/segmentation'
import type {
    CatchAll,
    Hemisphere,
    PartBound,
    PartCase,
    PartConfig,
    SegmentationBand,
    SegmentationConfig,
    SegmentationOffset,
    SegmentationPlaceholder,
    SegmentationServiceDetail,
} from '@/lib/types'

interface DraftBand extends SegmentationBand {
    _key: string
}

const nextKey = () => crypto.randomUUID()

const strip = (b: DraftBand): SegmentationBand => {
    const {_key, ...rest} = b
    void _key
    return pruneParts(rest)
}

/** Drop `parts` entries whose placeholder is no longer in the template (else the backend rejects). */
function pruneParts(band: SegmentationBand): SegmentationBand {
    if (!band.parts) return band
    const used = new Set(templatePlaceholders(band.template))
    const kept: Record<string, PartConfig> = {}
    for (const [ph, cfg] of Object.entries(band.parts)) if (used.has(ph)) kept[ph] = cfg
    return {...band, parts: Object.keys(kept).length ? kept : undefined}
}

/** Order-insensitive JSON, skipping `undefined`, so equal configs compare equal regardless of key
 *  order (band edits reshuffle keys) — used for the unsaved-changes check. */
function stableStringify(v: unknown): string {
    if (v === null || typeof v !== 'object') return JSON.stringify(v) ?? 'null'
    if (Array.isArray(v)) return `[${v.map(stableStringify).join(',')}]`
    const obj = v as Record<string, unknown>
    const keys = Object.keys(obj).filter((k) => obj[k] !== undefined).sort()
    return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(obj[k])}`).join(',')}}`
}

const KNOWN_PLACEHOLDERS = ['year', 'iso_year', 'quarter', 'season', 'month', 'week', 'day', 'weekday', 'daypart']

interface SegmentEditorProps {
    service: SegmentationServiceDetail
}

/**
 * Band-list editor for a calendar-segmentation service (feature 20). Edits a local draft and
 * commits the whole `SegmentationConfig` via `PUT …/config`. A clickable timeline previews the
 * buckets the bands produce; bands are shown indented under the band they subdivide (presentation
 * only — the model is a flat ordered list, array order = precedence).
 */
export function SegmentEditor({service}: SegmentEditorProps) {
    const {replaceConfig} = useTaggingMutations()
    const cfg = service.config

    const [rootTag, setRootTag] = useState(cfg.root_tag)
    const [hemisphere, setHemisphere] = useState<Hemisphere>(cfg.hemisphere ?? 'north')
    const [catchAll, setCatchAll] = useState<CatchAll | null>(cfg.catch_all)
    const [bands, setBands] = useState<DraftBand[]>(() => cfg.bands.map((b) => ({...b, _key: nextKey()})))
    const [selectedKey, setSelectedKey] = useState<string | null>(null)

    // Resync from server after a save / external edit.
    const serverKey = JSON.stringify(cfg)
    const [syncedKey, setSyncedKey] = useState(serverKey)
    if (serverKey !== syncedKey) {
        setRootTag(cfg.root_tag)
        setHemisphere(cfg.hemisphere ?? 'north')
        setCatchAll(cfg.catch_all)
        setBands(cfg.bands.map((b) => ({...b, _key: nextKey()})))
        setSyncedKey(serverKey)
    }

    const draft: SegmentationConfig = useMemo(
        () => ({version: 1, root_tag: rootTag, hemisphere, catch_all: catchAll, bands: bands.map(strip)}),
        [rootTag, hemisphere, catchAll, bands],
    )
    // Compare canonically against the server config normalized the same way `draft` is (pruned
    // parts, hemisphere default), so a pristine load — and an add-then-remove round-trip — is clean.
    const serverNormalized: SegmentationConfig = useMemo(
        () => ({
            version: 1,
            root_tag: cfg.root_tag,
            hemisphere: cfg.hemisphere ?? 'north',
            catch_all: cfg.catch_all,
            bands: cfg.bands.map(pruneParts)
        }),
        [cfg],
    )
    const dirty = stableStringify(draft) !== stableStringify(serverNormalized)
    const depths = useMemo(() => displayDepths(bands), [bands])
    const lints = useMemo(() => lintConfig(draft), [draft])

    const save = () => {
        replaceConfig.mutate({id: service.id, config: draft}, {onError: (err) => toast.error(apiErrorMessage(err))})
    }
    const reset = () => {
        setRootTag(cfg.root_tag)
        setHemisphere(cfg.hemisphere ?? 'north')
        setCatchAll(cfg.catch_all)
        setBands(cfg.bands.map((b) => ({...b, _key: nextKey()})))
    }

    const patchBand = (key: string, patch: Partial<SegmentationBand>) =>
        setBands((bs) => bs.map((b) => (b._key === key ? {...b, ...patch} : b)))

    const addBand = (base?: Partial<SegmentationBand>, at?: number) => {
        const band: DraftBand = {...newBand(), ...base, _key: nextKey()}
        setBands((bs) => {
            const next = [...bs]
            next.splice(at ?? next.length, 0, band)
            return next
        })
        setSelectedKey(band._key)
    }

    const sensors = useSensors(useSensor(PointerSensor, {activationConstraint: {distance: 4}}))
    const onDragEnd = (e: DragEndEvent) => {
        const {active, over} = e
        if (!over || active.id === over.id) return
        setBands((bs) => {
            const from = bs.findIndex((b) => b._key === active.id)
            const to = bs.findIndex((b) => b._key === over.id)
            if (from === -1 || to === -1) return bs
            const next = [...bs]
            const [moved] = next.splice(from, 1)
            next.splice(to, 0, moved)
            return next
        })
    }

    return (
        <div className="space-y-4">
            {/* Service-level fields */}
            <div className="grid gap-3 rounded-lg border bg-muted/20 p-3 sm:grid-cols-2">
                <div className="space-y-1">
                    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Root tag</span>
                    <div className="flex items-center gap-2">
                        <span className="text-sm">{rootTag ? TagPath.toDisplay(rootTag) : <em className="text-muted-foreground">none</em>}</span>
                        <TagPicker onSelect={setRootTag} allowCreate triggerLabel="Change"/>
                    </div>
                    <p className="text-[11px] text-muted-foreground">Every band's tag hangs under this.</p>
                </div>
                <div className="space-y-1">
                    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Hemisphere</span>
                    <Select value={hemisphere} onValueChange={(v) => setHemisphere(v as Hemisphere)}>
                        <SelectTrigger className="h-8 w-40 text-sm"><SelectValue/></SelectTrigger>
                        <SelectContent>
                            <SelectItem value="north">Northern</SelectItem>
                            <SelectItem value="south">Southern</SelectItem>
                        </SelectContent>
                    </Select>
                    <p className="text-[11px] text-muted-foreground">Only affects <code>{'{season}'}</code> names.</p>
                </div>
                <div className="sm:col-span-2">
                    <CatchAllEditor value={catchAll} onChange={setCatchAll}/>
                </div>
            </div>

            {/* Timeline preview */}
            <Timeline config={draft} depths={depths} bandKeys={bands.map((b) => b._key)} selectedKey={selectedKey} onSelect={setSelectedKey}/>

            {/* Lints */}
            {lints.length > 0 && (
                <ul className="space-y-1">
                    {lints.map((l, i) => (
                        <li key={i}
                            className="flex items-start gap-1.5 rounded-md border border-amber-500/30 bg-amber-500/5 px-2 py-1.5 text-xs text-amber-600 dark:text-amber-400">
                            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
                            <span>{l.bandIndex !== null && <strong>Band #{l.bandIndex + 1}: </strong>}{l.message}</span>
                        </li>
                    ))}
                </ul>
            )}

            {/* Band list */}
            <div>
                <div className="mb-2 flex items-center justify-between">
                    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                        Bands <span className="normal-case">— first match wins (top = highest precedence)</span>
                    </span>
                    <Button variant="outline" size="sm" className="h-7 gap-1.5 text-xs" onClick={() => addBand(undefined, 0)}>
                        <Plus className="h-3.5 w-3.5"/>
                        Add band
                    </Button>
                </div>

                {bands.length === 0 && <p className="text-sm text-muted-foreground">No bands yet — add one to start partitioning by date.</p>}

                <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
                    <SortableContext items={bands.map((b) => b._key)} strategy={verticalListSortingStrategy}>
                        <div className="space-y-1.5">
                            {bands.map((band, i) => (
                                <BandRow
                                    key={band._key}
                                    band={band}
                                    index={i}
                                    depth={depths[i]}
                                    hemisphere={hemisphere}
                                    expanded={selectedKey === band._key}
                                    onToggleExpand={() => setSelectedKey(selectedKey === band._key ? null : band._key)}
                                    onChange={(patch) => patchBand(band._key, patch)}
                                    onDelete={() => setBands((bs) => bs.filter((b) => b._key !== band._key))}
                                    onSubdivide={() => addBand({from: band.from, to: band.to, template: `${band.template}.{month}`}, i)}
                                />
                            ))}
                        </div>
                    </SortableContext>
                </DndContext>
            </div>

            {dirty && (
                <div className="flex items-center gap-2 border-t pt-3">
                    <Button size="sm" className="h-8 gap-1.5" onClick={save} disabled={replaceConfig.isPending}>
                        <Save className="h-3.5 w-3.5"/>
                        Save segmentation
                    </Button>
                    <Button size="sm" variant="ghost" className="h-8 gap-1.5" onClick={reset} disabled={replaceConfig.isPending}>
                        <Undo2 className="h-3.5 w-3.5"/>
                        Reset
                    </Button>
                </div>
            )}
        </div>
    )
}

// ── Catch-all ───────────────────────────────────────────────────────────────

function CatchAllEditor({value, onChange}: { value: CatchAll | null; onChange: (v: CatchAll | null) => void }) {
    return (
        <div className="flex flex-wrap items-center gap-2 text-sm">
            <label className="flex items-center gap-1.5">
                <Switch checked={!!value} onCheckedChange={(on) => onChange(on ? {name: 'Unsorted', include_undated: true} : null)}/>
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Catch-all</span>
            </label>
            {value && (
                <>
                    <Input value={value.name} onChange={(e) => onChange({...value, name: e.target.value})} placeholder="Unsorted"
                           className="h-8 w-40 text-sm"/>
                    <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <Checkbox checked={value.include_undated} onCheckedChange={(c) => onChange({...value, include_undated: c === true})}/>
                        Include undated photos
                    </label>
                </>
            )}
        </div>
    )
}

// ── Timeline ──────────────────────────────────────────────────────────────────

const YEAR_MS = 365.25 * 24 * 3600 * 1000

function Timeline({
                      config,
                      depths,
                      bandKeys,
                      selectedKey,
                      onSelect,
                  }: {
    config: SegmentationConfig
    depths: number[]
    bandKeys: string[]
    selectedKey: string | null
    onSelect: (key: string) => void
}) {
    const bands = config.bands
    const {min, max} = useMemo(() => {
        const dates = bands.flatMap((b) => [b.from, b.to]).filter((x): x is string => !!x).map((s) => parseDate(s).getTime())
        const now = Date.now()
        let lo = Math.min(...dates, now - 5 * YEAR_MS)
        let hi = Math.max(...dates, now + YEAR_MS)
        if (!isFinite(lo) || !isFinite(hi) || lo >= hi) {
            lo = now - 5 * YEAR_MS
            hi = now + YEAR_MS
        }
        const pad = (hi - lo) * 0.04
        return {min: lo - pad, max: hi + pad}
    }, [bands])

    const span = max - min
    const pct = (t: number) => `${((t - min) / span) * 100}%`
    const rows = Math.max(1, ...depths.map((d) => d + 1))
    const buckets = useMemo(() => previewBuckets(config, new Date(min), new Date(max), config.hemisphere ?? 'north'), [config, min, max])
    const ticks = useMemo(() => yearTicks(min, max), [min, max])

    if (bands.length === 0) return null

    return (
        <div className="rounded-lg border bg-muted/20 p-3">
            <div className="mb-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">Timeline preview</div>
            <div className="relative" style={{height: `${rows * 22 + 18}px`}}>
                {ticks.map((t) => (
                    <div key={t} className="absolute top-0 bottom-4 border-l border-border/40" style={{left: pct(parseDate(`${t}-01-01`).getTime())}}>
                        <span className="absolute -bottom-4 -translate-x-1/2 text-[10px] text-muted-foreground">{t}</span>
                    </div>
                ))}
                {bands.map((b, i) => {
                    const left = b.from ? parseDate(b.from).getTime() : min
                    const right = b.to ? parseDate(b.to).getTime() : max
                    const disabled = b.enabled === false
                    const selected = bandKeys[i] === selectedKey
                    return (
                        <button
                            key={bandKeys[i]}
                            onClick={() => onSelect(bandKeys[i])}
                            title={`#${i + 1} ${b.template}`}
                            className={`absolute flex items-center overflow-hidden rounded px-1.5 text-[10px] font-medium ring-1 transition-colors ${
                                disabled
                                    ? 'bg-muted text-muted-foreground/60 ring-border'
                                    : selected
                                        ? 'bg-sky-500/30 text-sky-700 ring-sky-500 dark:text-sky-200'
                                        : 'bg-sky-500/15 text-sky-700 ring-sky-500/30 hover:bg-sky-500/25 dark:text-sky-300'
                            }`}
                            style={{left: pct(left), width: `calc(${pct(right)} - ${pct(left)})`, top: `${depths[i] * 22}px`, height: '18px'}}
                        >
                            <span className="truncate">{b.template}</span>
                        </button>
                    )
                })}
            </div>
            {buckets.length > 0 && (
                <div className="mt-2 flex flex-wrap gap-1">
                    {buckets.slice(0, 40).map((bk, i) => (
                        <button
                            key={i}
                            onClick={() => onSelect(bandKeys[bk.bandIndex])}
                            className={`rounded px-1.5 py-0.5 font-mono text-[10px] ${
                                bandKeys[bk.bandIndex] === selectedKey ? 'bg-sky-500/25 text-sky-700 dark:text-sky-200' : 'bg-muted text-muted-foreground hover:text-foreground'
                            }`}
                        >
                            {bk.path}
                        </button>
                    ))}
                    {buckets.length > 40 && <span className="self-center text-[10px] text-muted-foreground">+{buckets.length - 40} more</span>}
                </div>
            )}
        </div>
    )
}

function yearTicks(min: number, max: number): number[] {
    const y0 = new Date(min).getFullYear()
    const y1 = new Date(max).getFullYear()
    const step = y1 - y0 > 24 ? 5 : y1 - y0 > 10 ? 2 : 1
    const out: number[] = []
    for (let y = Math.ceil(y0 / step) * step; y <= y1; y += step) out.push(y)
    return out
}

// ── Band row + config ──────────────────────────────────────────────────────────

function BandRow({
                     band,
                     index,
                     depth,
                     hemisphere,
                     expanded,
                     onToggleExpand,
                     onChange,
                     onDelete,
                     onSubdivide,
                 }: {
    band: DraftBand
    index: number
    depth: number
    hemisphere: Hemisphere
    expanded: boolean
    onToggleExpand: () => void
    onChange: (patch: Partial<SegmentationBand>) => void
    onDelete: () => void
    onSubdivide: () => void
}) {
    const {attributes, listeners, setNodeRef, transform, transition, isDragging} = useSortable({id: band._key})
    const style = {transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.4 : 1, marginLeft: `${depth * 20}px`}
    const disabled = band.enabled === false

    return (
        <div ref={setNodeRef} style={style} className={`rounded-md border ${expanded ? 'border-primary/40 bg-primary/5' : ''}`}>
            <div className="flex items-center gap-2 px-2 py-1.5 text-sm">
                <button className="cursor-grab touch-none text-muted-foreground/60 hover:text-foreground" {...attributes} {...listeners}
                        aria-label="Drag to reorder">
                    <GripVertical className="h-3.5 w-3.5"/>
                </button>
                <button onClick={onToggleExpand} className="text-muted-foreground hover:text-foreground">
                    {expanded ? <ChevronDown className="h-3.5 w-3.5"/> : <ChevronRight className="h-3.5 w-3.5"/>}
                </button>
                <span className="text-[11px] tabular-nums text-muted-foreground">#{index + 1}</span>
                <button onClick={onToggleExpand} className={`flex-1 text-left ${disabled ? 'text-muted-foreground line-through' : ''}`}>
                    <code className="font-mono text-xs">{band.template || '—'}</code>
                    <span className="ml-2 text-[11px] text-muted-foreground">{rangeLabel(band)}</span>
                </button>
                <Switch checked={!disabled} onCheckedChange={(on) => onChange({enabled: on})} aria-label="Enabled"/>
                <Button variant="ghost" size="icon" className="h-6 w-6 text-muted-foreground hover:text-foreground" onClick={onSubdivide}
                        title="Add a subdivision band for this range" aria-label="Subdivide">
                    <Scissors className="h-3.5 w-3.5"/>
                </Button>
                <Button variant="ghost" size="icon" className="h-6 w-6 text-muted-foreground hover:text-destructive" onClick={onDelete}
                        aria-label="Delete band">
                    <Trash2 className="h-3.5 w-3.5"/>
                </Button>
            </div>
            {expanded && <BandConfig band={band} hemisphere={hemisphere} onChange={onChange}/>}
        </div>
    )
}

function rangeLabel(band: SegmentationBand): string {
    return `${band.from ?? '−∞'} → ${band.to ?? '+∞'}`
}

function BandConfig({band, hemisphere, onChange}: {
    band: SegmentationBand;
    hemisphere: Hemisphere;
    onChange: (p: Partial<SegmentationBand>) => void
}) {
    const placeholders = templatePlaceholders(band.template).filter((p): p is SegmentationPlaceholder => KNOWN_PLACEHOLDERS.includes(p))
    const [showOffset, setShowOffset] = useState(!!band.offset)
    const sample = renderBandPath(band, new Date(), hemisphere)

    return (
        <div className="space-y-3 border-t px-3 py-3">
            <div className="flex flex-wrap items-center gap-3">
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Range</span>
                <DateRangePicker
                    mode="date"
                    from={band.from ?? ''}
                    to={band.to ?? ''}
                    onChange={(from, to) => onChange({from: from || null, to: to || null})}
                    placeholder="Any date"
                />
                <span className="text-[11px] text-muted-foreground">
                    Half-open [from, to) — the <strong>To</strong> day is not included (To = 2010-01-01 stops at 2009-12-31).
                    Set a single end (Start/End toggle) for open-ended.
                </span>
            </div>

            <div className="space-y-1">
                <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Template</span>
                <Input value={band.template} onChange={(e) => onChange({template: e.target.value})} placeholder="{year}.{month}"
                       className="h-8 font-mono text-sm"/>
                <p className="text-[11px] text-muted-foreground">
                    Placeholders: <code>{'{year} {iso_year} {quarter} {season} {month} {week} {day} {weekday} {daypart}'}</code>. <code>.</code> separates
                    sub-tag levels.
                    {sample && <> Sample today → <code className="text-foreground">{sample}</code></>}
                </p>
            </div>

            {placeholders.length > 0 && (
                <div className="space-y-2">
                    <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Placeholder formatting</span>
                    {placeholders.map((ph) => (
                        <PartEditor key={ph} placeholder={ph} value={band.parts?.[ph]}
                                    onChange={(part) => onChange({parts: mergePart(band.parts, ph, part)})}/>
                    ))}
                </div>
            )}

            <div>
                <button onClick={() => setShowOffset((v) => !v)}
                        className="flex items-center gap-1 text-xs font-medium uppercase tracking-wide text-muted-foreground hover:text-foreground">
                    {showOffset ? <ChevronDown className="h-3 w-3"/> : <ChevronRight className="h-3 w-3"/>}
                    Offset (boundary shift)
                </button>
                {showOffset && <OffsetEditor value={band.offset} onChange={(o) => onChange({offset: o})}/>}
            </div>
        </div>
    )
}

function mergePart(parts: Record<string, PartConfig> | undefined, ph: string, part: PartConfig | undefined): Record<string, PartConfig> {
    const next = {...(parts ?? {})}
    if (!part || (part.stride === undefined && (!part.format || Object.keys(part.format).length === 0))) {
        delete next[ph]
    } else {
        next[ph] = part
    }
    return next
}

function PartEditor({placeholder, value, onChange}: {
    placeholder: SegmentationPlaceholder;
    value: PartConfig | undefined;
    onChange: (p: PartConfig | undefined) => void
}) {
    const named = NAMED_PLACEHOLDERS.has(placeholder)
    const fmt = value?.format ?? {}
    const stride = value?.stride ?? 1
    const setFmt = (patch: Partial<PartConfig['format']>) => {
        const nextFmt = {...fmt, ...patch}
        ;(Object.keys(nextFmt) as (keyof typeof nextFmt)[]).forEach((k) => nextFmt[k] === undefined && delete nextFmt[k])
        onChange({stride: value?.stride, format: nextFmt})
    }
    const setStride = (n: number) => onChange({format: value?.format, stride: n <= 1 ? undefined : n})

    // Effective render kind: `numeric` is tri-state (unset = the field's §4.1 default). The toggle
    // below always writes an explicit boolean so a numeric-default field like `month` can be set to
    // names (otherwise unsetting fell back to numeric and names were unreachable).
    const effectiveNumeric = fmt.numeric ?? !placeholderDefaultsToName(placeholder)
    const asName = named && !effectiveNumeric
    const setNumeric = (isNumeric: boolean) =>
        isNumeric ? setFmt({numeric: true, abbrev: undefined, case: undefined}) : setFmt({numeric: false, pad: undefined})

    return (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-md border bg-background px-2.5 py-2 text-xs">
            <code className="font-mono font-medium text-foreground">{`{${placeholder}}`}</code>

            <label className="flex items-center gap-1 text-muted-foreground">
                stride
                <NumberInput value={stride} min={1} onChange={(e) => setStride(Number(e.target.value) || 1)} className="h-7 w-20"/>
            </label>

            {named && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    format
                    <Select value={effectiveNumeric ? 'numeric' : 'name'} onValueChange={(v) => setNumeric(v === 'numeric')}>
                        <SelectTrigger className="h-7 w-24 text-xs"><SelectValue/></SelectTrigger>
                        <SelectContent>
                            <SelectItem value="name">name</SelectItem>
                            <SelectItem value="numeric">numeric</SelectItem>
                        </SelectContent>
                    </Select>
                </label>
            )}
            {asName && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    <Checkbox checked={fmt.abbrev === true} onCheckedChange={(c) => setFmt({abbrev: c === true ? true : undefined})}/>
                    short
                </label>
            )}
            {!asName && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    pad
                    <NumberInput value={fmt.pad ?? 0} min={0} max={4} onChange={(e) => setFmt({pad: Number(e.target.value) || undefined})}
                                 className="h-7 w-16"/>
                </label>
            )}
            {asName && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    case
                    <Select value={fmt.case ?? 'pascal'} onValueChange={(v) => setFmt({case: v === 'pascal' ? undefined : (v as PartCase)})}>
                        <SelectTrigger className="h-7 w-24 text-xs"><SelectValue/></SelectTrigger>
                        <SelectContent>
                            <SelectItem value="pascal">Pascal</SelectItem>
                            <SelectItem value="lower">lower</SelectItem>
                            <SelectItem value="upper">UPPER</SelectItem>
                        </SelectContent>
                    </Select>
                </label>
            )}
            {stride > 1 && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    block
                    <Select value={fmt.bound ?? 'start'} onValueChange={(v) => setFmt({bound: v === 'start' ? undefined : (v as PartBound)})}>
                        <SelectTrigger className="h-7 w-24 text-xs"><SelectValue/></SelectTrigger>
                        <SelectContent>
                            <SelectItem value="start">start</SelectItem>
                            <SelectItem value="end">end</SelectItem>
                            <SelectItem value="range">range</SelectItem>
                        </SelectContent>
                    </Select>
                </label>
            )}
            {stride > 1 && fmt.bound === 'range' && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    sep
                    <Input value={fmt.range_sep ?? '_'} onChange={(e) => setFmt({range_sep: e.target.value || undefined})}
                           className="h-7 w-12 text-xs"/>
                </label>
            )}
            {stride > 1 && (fmt.bound === 'end' || fmt.bound === 'range') && (
                <label className="flex items-center gap-1 text-muted-foreground">
                    <Checkbox checked={fmt.inclusive_end === true} onCheckedChange={(c) => setFmt({inclusive_end: c === true ? true : undefined})}/>
                    inclusive end
                </label>
            )}
        </div>
    )
}

function OffsetEditor({value, onChange}: { value: SegmentationOffset | undefined; onChange: (o: SegmentationOffset | undefined) => void }) {
    const set = (k: keyof SegmentationOffset, v: number) => {
        const next: SegmentationOffset = {...value, [k]: v || undefined}
        ;(Object.keys(next) as (keyof SegmentationOffset)[]).forEach((key) => next[key] === undefined && delete next[key])
        onChange(Object.keys(next).length ? next : undefined)
    }
    const fields: (keyof SegmentationOffset)[] = ['months', 'days', 'hours', 'minutes']
    return (
        <div className="mt-1.5 flex flex-wrap gap-3">
            {fields.map((f) => (
                <label key={f} className="flex items-center gap-1 text-xs text-muted-foreground">
                    {f}
                    <NumberInput value={value?.[f] ?? 0} min={0} onChange={(e) => set(f, Number(e.target.value) || 0)} className="h-7 w-16"/>
                </label>
            ))}
            <span className="self-center text-[11px] text-muted-foreground">Subtracted from capture time before projecting (e.g. 4 h photographic day).</span>
        </div>
    )
}
