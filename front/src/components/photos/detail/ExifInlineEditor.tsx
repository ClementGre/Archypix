import {useRef, useState} from 'react'
import {ChevronDown, Loader2, RotateCcw, Save, Send, UserRound} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import {Section} from './Section'
import {FieldLabel} from './FieldLabel'
import {DateTimePickerPopover, formatNaive} from './DateTimePickerPopover'
import {dateSuggestions} from '@/lib/dateSuggestions'
import {GpsPickerPopover} from './GpsPickerPopover'
import {type FieldState, FieldStateHint} from './FieldStateHint'
import {cn, formatDuration, isAudioMime, isVideoMime} from '@/lib/utils'
import {formatAccuracy} from '@/lib/gpsInterpolation'
import type {ExifField, ExifSyncStatus, PictureDetail} from '@/lib/types'
import {type ExifDraft, GPS_KEYS, type useExifDraft} from '@/hooks/useExifDraft'

/** A read-only metadata row (label + value), matching the editable rows' layout. */
export function ReadOnlyRow({label, value}: { label: string; value: string }) {
    return (
        <div className="flex min-h-[1.4rem] items-center gap-1">
            <div className="w-3 shrink-0"/>
            <div className="w-24 shrink-0"><FieldLabel>{label}</FieldLabel></div>
            <span className="flex-1 truncate text-right text-xs">{value}</span>
            <div className="w-4 shrink-0"/>
        </div>
    )
}

/** Feature 33 §11 — one label per state, so no raw enum ever reaches a user. */
const SYNC_BADGE: Record<ExifSyncStatus, string> = {
    synced: 'synced',
    pending: 'pending',
    // An internal worklist marker: the drain owes this row a job, which reads as "pending".
    pending_job_creation: 'pending',
    extracting: 'reading metadata',
    extract_failed: 'metadata unread',
    unsupported_mime: 'n/a',
    unsupported_file: 'unreadable',
    write_failed: 'write error',
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
                         state,
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
    /** Override / file-diff annotation: tints the value and explains it in a hover popup. */
    state?: FieldState
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
            <div className="w-24 shrink-0"><FieldLabel>{label}</FieldLabel></div>
            <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
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
                    <FieldStateHint state={state}>
                        <button
                            onClick={startEdit}
                            disabled={!canEdit}
                            className={cn(
                                'min-w-0 truncate rounded px-1 text-right text-xs',
                                !value && 'text-muted-foreground',
                                canEdit && 'cursor-pointer transition-colors',
                                canEdit && !state && 'hover:bg-muted',
                            )}
                        >
                            {display}
                        </button>
                    </FieldStateHint>
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
                         state,
                     }: {
    num: string
    den: string
    isDirty: boolean
    onChangeNum: (v: string) => void
    onChangeDen: (v: string) => void
    onReset: () => void
    canEdit: boolean
    state?: FieldState
}) {
    const [editing, setEditing] = useState(false)
    const display = num && den ? `${num}/${den} s` : '—'

    function handleBlur(e: React.FocusEvent<HTMLDivElement>) {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setEditing(false)
    }

    return (
        <div className="group flex min-h-[1.4rem] items-center gap-1 text-sm">
            <DirtyDot isDirty={isDirty}/>
            <div className="w-24 shrink-0"><FieldLabel>Exposure</FieldLabel></div>
            <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
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
                    <FieldStateHint state={state}>
                        <button
                            onClick={() => canEdit && setEditing(true)}
                            disabled={!canEdit}
                            className={cn(
                                'truncate rounded px-1 text-right text-xs',
                                display === '—' && 'text-muted-foreground',
                                canEdit && 'cursor-pointer transition-colors',
                                canEdit && !state && 'hover:bg-muted',
                            )}
                        >
                            {display}
                        </button>
                    </FieldStateHint>
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
    const {
        draft,
        initialDraft,
        isDirty,
        isSaving,
        owned,
        allowExifEdit,
        overriddenKeys,
        originDraft,
        fileDraft,
        hasFileSnapshot,
        revertFieldsToFile,
        set,
        setGps,
        reset,
        resetGps,
        save,
        removeOverride,
        retrySync,
        retrying,
        revertToFile,
        reverting,
        reextract,
        reextracting,
    } = exif
    // Video/audio: ffprobe metadata, not photographic EXIF. Hide the camera-only rows (focal length,
    // aperture, ISO, exposure) and surface a read-only media-info block instead. Edits are DB-only
    // (the worker can't rewrite container metadata): the first edit stamps `unsupported_mime`.
    const isMedia = isVideoMime(picture.mime_type) || isAudioMime(picture.mime_type)
    const ex = (picture.exif_data ?? {}) as Record<string, unknown>
    const num = (v: unknown): number | null => (typeof v === 'number' && isFinite(v) ? v : null)
    const str = (v: unknown): string | null => (typeof v === 'string' && v ? v : null)
    const mediaDuration = num(ex.duration_s)
    const mediaFps = num(ex.frame_rate)
    const mediaVideoCodec = str(ex.video_codec)
    const mediaAudioCodec = str(ex.audio_codec)
    // A received picture whose incoming share authorises EXIF editing offers two save modes:
    // "Suggest to owner" (propose — propagates to everyone) vs "Just for me" (private local override).
    const canPropose = !owned && allowExifEdit
    // The holder can always edit: owned pictures write through to the file; received pictures get a
    // recipient-local override.
    const canEdit = true

    const syncLabel = SYNC_BADGE[picture.exif_sync_status]
    const writeFailed = owned && picture.exif_sync_status === 'write_failed'
    // "We never read this file" (feature 33 §4.2): edits are allowed and repair the row, and a
    // re-extract can settle it. Unlike `unsupported_file`, offering a retry here is not a lie.
    const extractFailed = owned && picture.exif_sync_status === 'extract_failed'
    const fileExif = (picture.file_exif ?? {}) as Record<string, unknown>

    const dirty = (k: keyof ExifDraft) => draft[k] !== initialDraft[k]
    const isOverridden = (...keys: Array<keyof ExifDraft>) => !owned && keys.some((k) => overriddenKeys.has(k))
    // A field whose stored value no longer matches the file (feature 31 §8). Numbers are compared
    // with a tolerance: EXIF keeps GPS and exposure as rationals, so a round-trip drifts slightly
    // and an exact string compare would badge every coordinate as different.
    const differsFromFile = (k: keyof ExifDraft) => {
        const dbv = initialDraft[k]
        const fv = fileExif[k]
        const fvs = fv == null ? '' : String(fv)
        if (dbv === fvs) return false
        const a = Number(dbv)
        const b = Number(fvs)
        if (dbv !== '' && fvs !== '' && !isNaN(a) && !isNaN(b)) return Math.abs(a - b) > 1e-5
        return true
    }
    /**
     * The annotation a row carries, if any: a recipient's local override, or (owned, `write_failed`)
     * a value that never reached the file. Rendered as a tint + hover popup rather than an inline
     * badge — these rows are narrow and a badge pushed the value out of the panel.
     */
    const fieldState = (...keys: Array<keyof ExifDraft>): FieldState | undefined => {
        if (isOverridden(...keys)) {
            return {
                tone: 'override',
                reference: keys.map((k) => originDraft[k]).filter(Boolean).join(', '),
                onRevert: () => removeOverride(...(keys as ExifField[])),
            }
        }
        if (writeFailed && keys.some(differsFromFile)) {
            return {
                tone: 'diff',
                reference: keys.map((k) => fileDraft[k]).filter(Boolean).join(', '),
                onRevert: () => revertFieldsToFile(...(keys as ExifField[])),
            }
        }
        return undefined
    }

    const gpsDisplay =
        draft.gps_lat && draft.gps_lng
            ? `${parseFloat(draft.gps_lat).toFixed(4)}, ${parseFloat(draft.gps_lng).toFixed(4)}${draft.gps_alt ? ` · ${draft.gps_alt} m` : ''}${draft.gps_accuracy_m ? ` · ${formatAccuracy(parseFloat(draft.gps_accuracy_m))}` : ''}`
            : '—'
    const gpsIsDirty = GPS_KEYS.some(dirty)
    const gpsState = fieldState(...GPS_KEYS)
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
        // Media tech fields rendered in their own block below.
        'duration_s',
        'frame_rate',
        'video_codec',
        'audio_codec',
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
                <div className="flex items-center justify-end gap-1 flex-wrap">
                    <Badge
                        variant="outline"
                        className={cn(
                            'h-5 px-1.5 text-[10px]',
                            isDirty
                                ? 'border-primary text-primary'
                                : !owned && overriddenKeys.size > 0
                                    ? 'border-amber-500 text-amber-500'
                                    : owned && (picture.exif_sync_status === 'pending'
                                        || picture.exif_sync_status === 'pending_job_creation'
                                        || picture.exif_sync_status === 'extracting')
                                        ? 'border-yellow-500 text-yellow-500'
                                        : owned && picture.exif_sync_status === 'write_failed'
                                            ? 'border-red-500 text-red-500'
                                            : extractFailed
                                                ? 'border-amber-500 text-amber-500'
                                                : undefined,
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
                    {extractFailed && !isDirty && (
                        <Button
                            variant="ghost"
                            size="sm"
                            className="h-6 px-2 text-[10px]"
                            onClick={reextract}
                            disabled={reextracting}
                            title="Read this file's metadata again"
                        >
                            Re-extract
                        </Button>
                    )}
                    {writeFailed && !isDirty && (
                        <>
                            <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-2 text-[10px]"
                                onClick={retrySync}
                                disabled={retrying || reverting}
                            >
                                Retry
                            </Button>
                            {/* No snapshot ⇒ nothing to revert to: the endpoint can only 409. */}
                            <Button
                                variant="ghost"
                                size="sm"
                                className="h-6 px-2 text-[10px]"
                                onClick={revertToFile}
                                disabled={reverting || retrying || !hasFileSnapshot}
                                title={
                                    hasFileSnapshot
                                        ? 'Revert the stored exif metadata to the one of the file'
                                        : 'No EXIF has been read from this file yet — nothing to revert to'
                                }
                            >
                                Revert
                            </Button>
                        </>
                    )}
                    {isDirty &&
                        (canPropose ? (
                            <DropdownMenu>
                                <DropdownMenuTrigger asChild>
                                    <Button
                                        variant="ghost"
                                        size="icon"
                                        className="h-6 w-10 text-primary"
                                        disabled={isSaving}
                                        title="Save EXIF changes"
                                    >
                                        {isSaving ? (
                                            <Loader2 className="h-3.5 w-3.5 animate-spin"/>
                                        ) : (
                                            <span className="flex items-center">
                                                <Save className="h-3.5 w-3.5"/>
                                                <ChevronDown className="h-2.5 w-2.5"/>
                                            </span>
                                        )}
                                    </Button>
                                </DropdownMenuTrigger>
                                <DropdownMenuContent align="end" className="w-56">
                                    <DropdownMenuItem onClick={() => save('propose')} className="gap-2">
                                        <Send className="h-3.5 w-3.5"/>
                                        <span className="flex flex-col">
                                            <span>Suggest to owner</span>
                                            <span className="text-[10px] text-muted-foreground">Applies for everyone</span>
                                        </span>
                                    </DropdownMenuItem>
                                    <DropdownMenuItem onClick={() => save('local')} className="gap-2">
                                        <UserRound className="h-3.5 w-3.5"/>
                                        <span className="flex flex-col">
                                            <span>Just for me</span>
                                            <span className="text-[10px] text-muted-foreground">Private local override</span>
                                        </span>
                                    </DropdownMenuItem>
                                </DropdownMenuContent>
                            </DropdownMenu>
                        ) : (
                            <Button
                                variant="ghost"
                                size="icon"
                                className="h-6 w-6 text-primary"
                                onClick={() => save('local')}
                                disabled={isSaving}
                                title={owned ? 'Save EXIF changes' : 'Save local overrides'}
                            >
                                {isSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <Save className="h-3.5 w-3.5"/>}
                            </Button>
                        ))}
                </div>
            }
        >
            <div className="space-y-0.5">
                {/* Captured at — date/time picker */}
                <div className="group flex min-h-[1.4rem] items-center gap-1 text-sm">
                    <DirtyDot isDirty={dirty('captured_at')}/>
                    <div className="w-24 shrink-0"><FieldLabel>Captured at</FieldLabel></div>
                    <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                        {canEdit ? (
                            <FieldStateHint state={fieldState('captured_at')}>
                                <DateTimePickerPopover
                                    value={draft.captured_at || null}
                                    onChange={(v) => set('captured_at', v ?? '')}
                                    // When the capture date is empty, offer "From filename / file date /
                                    // upload" prefills (feature 30 §6) so pictures get fixed inline.
                                    suggestions={draft.captured_at ? undefined : dateSuggestions(picture)}
                                >
                                    <button
                                        className={cn('truncate rounded px-1 text-right text-xs transition-colors', !fieldState('captured_at') && 'hover:bg-muted', !draft.captured_at && 'text-muted-foreground')}>
                                        {formatNaive(draft.captured_at || null) || 'Set date'}
                                    </button>
                                </DateTimePickerPopover>
                            </FieldStateHint>
                        ) : (
                            <span
                                className="truncate text-right text-xs text-muted-foreground">{formatNaive(draft.captured_at || null) || 'Not set'}</span>
                        )}
                    </div>
                    <ResetSlot isDirty={dirty('captured_at')} onReset={() => reset('captured_at')}/>
                </div>

                {/* GPS — map picker */}
                <div className="group flex min-h-[1.4rem] items-center gap-1">
                    <DirtyDot isDirty={gpsIsDirty}/>
                    <div className="w-24 shrink-0"><FieldLabel>GPS</FieldLabel></div>
                    <div className="flex min-w-0 flex-1 items-center justify-end gap-1">
                        {canEdit ? (
                            <FieldStateHint state={gpsState}>
                                <GpsPickerPopover
                                    value={{lat: draft.gps_lat, lng: draft.gps_lng, alt: draft.gps_alt, accuracy: draft.gps_accuracy_m}}
                                    onChange={setGps}
                                >
                                    <button
                                        className={cn(
                                            'truncate rounded px-1 text-right text-xs transition-colors',
                                            !gpsState && 'hover:bg-muted',
                                            gpsDisplay === '—' && 'text-muted-foreground',
                                        )}
                                    >
                                        {gpsDisplay}
                                    </button>
                                </GpsPickerPopover>
                            </FieldStateHint>
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
                    state={fieldState('camera_brand')}
                />
                <EditableRow
                    label="Camera model"
                    value={draft.camera_model}
                    isDirty={dirty('camera_model')}
                    onReset={() => reset('camera_model')}
                    onChange={(v) => set('camera_model', v)}
                    placeholder="EOS R5"
                    canEdit={canEdit}
                    state={fieldState('camera_model')}
                />
                {/* Photographic-only fields — hidden for video/audio (no lens/exposure metadata). */}
                {!isMedia && (
                    <>
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
                            state={fieldState('focal_length_mm')}
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
                            state={fieldState('f_number')}
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
                            state={fieldState('iso_speed')}
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
                            state={fieldState('exposure_time_num', 'exposure_time_den')}
                        />
                    </>
                )}

                {/* Media (video/audio) technical metadata — read-only (ffprobe). */}
                {isMedia && (mediaDuration != null || mediaVideoCodec || mediaAudioCodec || mediaFps != null) && (
                    <>
                        {mediaDuration != null && <ReadOnlyRow label="Duration" value={formatDuration(mediaDuration)}/>}
                        {mediaFps != null && <ReadOnlyRow label="Frame rate" value={`${mediaFps} fps`}/>}
                        {mediaVideoCodec && <ReadOnlyRow label="Video codec" value={mediaVideoCodec}/>}
                        {mediaAudioCodec && <ReadOnlyRow label="Audio codec" value={mediaAudioCodec}/>}
                    </>
                )}

                {/* Extra raw exif_data fields (read-only) */}
                {extraRows.map(([k, v]) => (
                    <div key={k} className="flex min-h-[1.75rem] items-center gap-1 text-sm">
                        <div className="w-3 shrink-0"/>
                        <div className="w-24 shrink-0"><FieldLabel>{k}</FieldLabel></div>
                        <span className="flex-1 truncate text-right text-sm">{v}</span>
                        <div className="w-4 shrink-0"/>
                    </div>
                ))}
            </div>
        </Section>
    )
}
