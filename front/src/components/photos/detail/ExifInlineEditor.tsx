import {type ReactNode, useRef, useState} from 'react'
import {Loader2, RotateCcw, Save} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Section} from './Section'
import {DateTimePickerPopover, formatNaive} from './DateTimePickerPopover'
import {GpsPickerPopover} from './GpsPickerPopover'
import {OverwrittenBadge} from './OverwrittenBadge'
import {cn} from '@/lib/utils'
import type {PictureDetail} from '@/lib/types'
import type {ExifDraft, useExifDraft} from '@/hooks/useExifDraft'

const SYNC_BADGE: Record<string, string> = {
    synced: 'synced',
    pending: 'pending',
    unsupported: 'n/a',
}

/** A small reset (↺) affordance that appears for dirty rows on hover. */
function ResetSlot({isDirty, onReset}: { isDirty: boolean; onReset: () => void }) {
    return (
        <div className="flex w-4 shrink-0 items-center justify-center">
            {isDirty && (
                <button
                    onClick={(e) => {
                        e.stopPropagation();
                        onReset()
                    }}
                    title="Reset"
                    className="text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100"
                >
                    <RotateCcw className="h-3 w-3"/>
                </button>
            )}
        </div>
    )
}

function DirtyDot({isDirty}: { isDirty: boolean }) {
    return (
        <div className="flex w-3 shrink-0 items-center justify-center">
            {isDirty && <div className="h-1.5 w-1.5 rounded-full bg-primary"/>}
        </div>
    )
}

/** Inline-editable text/number row with optional unit prefix/suffix. */
function EditableRow({
                         label,
                         value,
                         isDirty,
                         onReset,
                         onChange,
                         type = 'text',
                         placeholder,
                         step,
                         min,
                         max,
                         prefix,
                         suffix,
                         canEdit,
                         badge,
                     }: {
    label: string
    value: string
    isDirty: boolean
    onReset: () => void
    onChange: (v: string) => void
    type?: 'text' | 'number'
    placeholder?: string
    step?: string | number
    min?: number
    max?: number
    prefix?: string
    suffix?: string
    canEdit: boolean
    /** Optional indicator rendered before the value (e.g. an "overwritten" badge). */
    badge?: ReactNode
}) {
    const [editing, setEditing] = useState(false)
    const [inputVal, setInputVal] = useState(value)
    const ref = useRef<HTMLInputElement>(null)

    function startEdit() {
        if (!canEdit) return
        setInputVal(value)
        setEditing(true)
        setTimeout(() => ref.current?.select(), 0)
    }

    function commit() {
        setEditing(false)
        onChange(inputVal)
    }

    function handleKey(e: React.KeyboardEvent) {
        if (e.key === 'Enter') {
            e.preventDefault();
            commit()
        }
        if (e.key === 'Escape') {
            setEditing(false);
            setInputVal(value)
        }
    }

    const display = value ? `${prefix ?? ''}${value}${suffix ? ` ${suffix}` : ''}` : '—'

    return (
        <div className="group flex min-h-[1.4rem] items-center gap-1">
            <DirtyDot isDirty={isDirty}/>
            <span className="w-24 shrink-0 text-xs text-muted-foreground">{label}</span>
            <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                {!editing && badge}
                {editing ? (
                    <div className="flex w-full items-center rounded border border-input bg-background focus-within:ring-1 focus-within:ring-ring">
                        {prefix && <span className="pl-1.5 text-xs text-muted-foreground">{prefix}</span>}
                        <input
                            ref={ref}
                            type={type}
                            value={inputVal}
                            onChange={(e) => setInputVal(e.target.value)}
                            onBlur={commit}
                            onKeyDown={handleKey}
                            step={step}
                            min={min}
                            max={max}
                            placeholder={placeholder}
                            className="w-full bg-transparent px-1.5 py-0.5 text-right text-xs focus:outline-none"
                        />
                        {suffix && <span className="pr-1.5 text-xs text-muted-foreground">{suffix}</span>}
                    </div>
                ) : (
                    <button
                        onClick={startEdit}
                        disabled={!canEdit}
                        className={cn(
                            'min-w-0 truncate rounded px-1 text-right text-xs',
                            !value && 'text-muted-foreground',
                            canEdit && 'cursor-pointer transition-colors hover:bg-muted',
                        )}
                    >
                        {display}
                    </button>
                )}
            </div>
            <ResetSlot isDirty={isDirty} onReset={onReset}/>
        </div>
    )
}

/** Exposure (num / den) row — click-to-edit, like {@link EditableRow}. */
function ExposureRow({
                         num,
                         den,
                         isDirty,
                         onChangeNum,
                         onChangeDen,
                         onReset,
                         canEdit,
                         badge,
                     }: {
    num: string
    den: string
    isDirty: boolean
    onChangeNum: (v: string) => void
    onChangeDen: (v: string) => void
    onReset: () => void
    canEdit: boolean
    badge?: ReactNode
}) {
    const [editing, setEditing] = useState(false)
    const display = num && den ? `${num}/${den} s` : '—'

    function handleBlur(e: React.FocusEvent<HTMLDivElement>) {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setEditing(false)
    }

    return (
        <div className="group flex min-h-[1.4rem] items-center gap-1 text-sm">
            <DirtyDot isDirty={isDirty}/>
            <span className="w-24 shrink-0 text-xs text-muted-foreground">Exposure</span>
            <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                {!editing && badge}
                {editing ? (
                    <div className="flex items-center gap-1" onBlur={handleBlur}>
                        <input
                            type="number"
                            step={1}
                            placeholder="1"
                            autoFocus
                            value={num}
                            onChange={(e) => onChangeNum(e.target.value)}
                            className="w-12 rounded border border-input bg-background px-1 py-0.5 text-right text-xs focus:outline-none focus:ring-1 focus:ring-ring"
                        />
                        <span className="text-xs text-muted-foreground">/</span>
                        <input
                            type="number"
                            step={1}
                            placeholder="200"
                            value={den}
                            onChange={(e) => onChangeDen(e.target.value)}
                            className="w-14 rounded border border-input bg-background px-1 py-0.5 text-right text-xs focus:outline-none focus:ring-1 focus:ring-ring"
                        />
                        <span className="text-xs text-muted-foreground">s</span>
                    </div>
                ) : (
                    <button
                        onClick={() => canEdit && setEditing(true)}
                        disabled={!canEdit}
                        className={cn(
                            'truncate rounded px-1 text-right text-xs',
                            display === '—' && 'text-muted-foreground',
                            canEdit && 'cursor-pointer transition-colors hover:bg-muted',
                        )}
                    >
                        {display}
                    </button>
                )}
            </div>
            <ResetSlot isDirty={isDirty} onReset={onReset}/>
        </div>
    )
}

export function ExifInlineEditor({
                                     picture,
                                     exif,
                                 }: {
    picture: PictureDetail
    exif: ReturnType<typeof useExifDraft>
}) {
    const {draft, initialDraft, isDirty, isSaving, owned, overriddenKeys, set, setGps, reset, resetGps, save, removeOverride} = exif
    // The holder can always edit: owned pictures write through to the file; received pictures get a
    // recipient-local override.
    const canEdit = true

    const syncLabel = SYNC_BADGE[picture.exif_sync_status] ?? picture.exif_sync_status

    const dirty = (k: keyof ExifDraft) => draft[k] !== initialDraft[k]
    const isOverridden = (...keys: Array<keyof ExifDraft>) => !owned && keys.some((k) => overriddenKeys.has(k))
    /** "overwritten" indicator for a received-picture field (clears the override on ✕). */
    const overrideBadge = (...keys: Array<keyof ExifDraft>) =>
        isOverridden(...keys) ? <OverwrittenBadge onRemove={() => removeOverride(...keys)}/> : undefined

    const gpsDisplay =
        draft.gps_lat && draft.gps_lng
            ? `${parseFloat(draft.gps_lat).toFixed(4)}, ${parseFloat(draft.gps_lng).toFixed(4)}${draft.gps_alt ? ` · ${draft.gps_alt} m` : ''}`
            : '—'
    const gpsIsDirty = dirty('gps_lat') || dirty('gps_lng') || dirty('gps_alt')
    const expIsDirty = dirty('exposure_time_num') || dirty('exposure_time_den')

    // Raw exif_data fields not surfaced as dedicated rows (read-only).
    const known = [
        'camera_brand',
        'camera_model',
        'focal_length_mm',
        'f_number',
        'iso_speed',
        'exposure_time_num',
        'exposure_time_den',
        'orientation',
    ]
    const extraRows: Array<[string, string]> = []
    for (const [k, v] of Object.entries(picture.exif_data ?? {})) {
        if (known.includes(k) || v == null || typeof v === 'object') continue
        extraRows.push([k, String(v)])
    }

    return (
        <Section
            id="exif"
            title="EXIF"
            defaultOpen={false}
            action={
                <div className="flex items-center gap-1">
                    <Badge
                        variant="outline"
                        className={cn(
                            'h-5 px-1.5 text-[10px]',
                            isDirty
                                ? 'border-primary text-primary'
                                : !owned && overriddenKeys.size > 0
                                    ? 'border-amber-500 text-amber-500'
                                    : owned && picture.exif_sync_status === 'pending' && 'border-yellow-500 text-yellow-500',
                        )}
                    >
                        {isDirty
                            ? 'modified'
                            : !owned
                                ? overriddenKeys.size > 0
                                    ? 'overridden'
                                    : 'local'
                                : syncLabel}
                    </Badge>
                    {isDirty && (
                        <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6 text-primary"
                            onClick={save}
                            disabled={isSaving}
                            title={owned ? 'Save EXIF changes' : 'Save local overrides'}
                        >
                            {isSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <Save className="h-3.5 w-3.5"/>}
                        </Button>
                    )}
                </div>
            }
        >
            <div className="space-y-0.5">
                {/* Captured at — date/time picker */}
                <div className="group flex min-h-[1.4rem] items-center gap-1 text-sm">
                    <DirtyDot isDirty={dirty('captured_at')}/>
                    <span className="w-24 shrink-0 text-xs text-muted-foreground">Captured at</span>
                    <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                        {overrideBadge('captured_at')}
                        {canEdit ? (
                            <DateTimePickerPopover
                                value={draft.captured_at || null}
                                onChange={(v) => set('captured_at', v ?? '')}
                            >
                                <button className="truncate rounded px-1 text-right text-xs transition-colors hover:bg-muted">
                                    {formatNaive(draft.captured_at || null)}
                                </button>
                            </DateTimePickerPopover>
                        ) : (
                            <span className="truncate text-right text-xs">{formatNaive(draft.captured_at || null)}</span>
                        )}
                    </div>
                    <ResetSlot isDirty={dirty('captured_at')} onReset={() => reset('captured_at')}/>
                </div>

                {/* GPS — map picker */}
                <div className="group flex min-h-[1.4rem] items-center gap-1">
                    <DirtyDot isDirty={gpsIsDirty}/>
                    <span className="w-24 shrink-0 text-xs text-muted-foreground">GPS</span>
                    <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                        {overrideBadge('gps_lat', 'gps_lng', 'gps_alt')}
                        {canEdit ? (
                            <GpsPickerPopover
                                value={{lat: draft.gps_lat, lng: draft.gps_lng, alt: draft.gps_alt}}
                                onChange={({lat, lng, alt}) => setGps(lat, lng, alt)}
                            >
                                <button
                                    className={cn(
                                        'truncate rounded px-1 text-right text-xs transition-colors hover:bg-muted',
                                        gpsDisplay === '—' && 'text-muted-foreground',
                                    )}
                                >
                                    {gpsDisplay}
                                </button>
                            </GpsPickerPopover>
                        ) : (
                            <span className={cn('truncate text-right text-xs', gpsDisplay === '—' && 'text-muted-foreground')}>
                                {gpsDisplay}
                            </span>
                        )}
                    </div>
                    <ResetSlot isDirty={gpsIsDirty} onReset={resetGps}/>
                </div>

                <EditableRow
                    label="Camera brand"
                    value={draft.camera_brand}
                    isDirty={dirty('camera_brand')}
                    onReset={() => reset('camera_brand')}
                    onChange={(v) => set('camera_brand', v)}
                    placeholder="Canon"
                    canEdit={canEdit}
                    badge={overrideBadge('camera_brand')}
                />
                <EditableRow
                    label="Camera model"
                    value={draft.camera_model}
                    isDirty={dirty('camera_model')}
                    onReset={() => reset('camera_model')}
                    onChange={(v) => set('camera_model', v)}
                    placeholder="EOS R5"
                    canEdit={canEdit}
                    badge={overrideBadge('camera_model')}
                />
                <EditableRow
                    label="Focal length"
                    value={draft.focal_length_mm}
                    isDirty={dirty('focal_length_mm')}
                    onReset={() => reset('focal_length_mm')}
                    onChange={(v) => set('focal_length_mm', v)}
                    type="number"
                    step="any"
                    placeholder="50"
                    suffix="mm"
                    canEdit={canEdit}
                    badge={overrideBadge('focal_length_mm')}
                />
                <EditableRow
                    label="Aperture"
                    value={draft.f_number}
                    isDirty={dirty('f_number')}
                    onReset={() => reset('f_number')}
                    onChange={(v) => set('f_number', v)}
                    type="number"
                    step="any"
                    placeholder="1.8"
                    prefix="f/"
                    canEdit={canEdit}
                    badge={overrideBadge('f_number')}
                />
                <EditableRow
                    label="ISO"
                    value={draft.iso_speed}
                    isDirty={dirty('iso_speed')}
                    onReset={() => reset('iso_speed')}
                    onChange={(v) => set('iso_speed', v)}
                    type="number"
                    step={1}
                    placeholder="400"
                    prefix="ISO "
                    canEdit={canEdit}
                    badge={overrideBadge('iso_speed')}
                />

                <ExposureRow
                    num={draft.exposure_time_num}
                    den={draft.exposure_time_den}
                    isDirty={expIsDirty}
                    onChangeNum={(v) => set('exposure_time_num', v)}
                    onChangeDen={(v) => set('exposure_time_den', v)}
                    onReset={() => {
                        reset('exposure_time_num');
                        reset('exposure_time_den')
                    }}
                    canEdit={canEdit}
                    badge={overrideBadge('exposure_time_num', 'exposure_time_den')}
                />

                {/* Extra raw exif_data fields (read-only) */}
                {extraRows.map(([k, v]) => (
                    <div key={k} className="flex min-h-[1.75rem] items-center gap-1 text-sm">
                        <div className="w-3 shrink-0"/>
                        <span className="w-24 shrink-0 text-xs text-muted-foreground">{k}</span>
                        <span className="flex-1 truncate text-right text-sm">{v}</span>
                        <div className="w-4 shrink-0"/>
                    </div>
                ))}
            </div>
        </Section>
    )
}
