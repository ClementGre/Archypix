// Capture-date fix surface (feature 30 §6) for a single target, shown as a non-collapsible pane. A
// centered inline calendar (on a consistent full-width background) stays for manual tweaking; below it
// the suggestion chips — or, while picking references, a timeline of the reference dates in their
// place. Received targets choose local vs propose.

import {useEffect, useRef, useState} from 'react'
import {FileClock, FileText, Loader2, Upload, Users} from 'lucide-react'
import {toast} from 'sonner'
import {Calendar} from '@/components/ui/calendar'
import {Input} from '@/components/ui/input'
import {buildNaive, parseNaive} from '@/lib/fixDate'
import {formatNaive} from '@/components/photos/detail/DateTimePickerPopover'
import {DateTimelinePreview} from './DateTimelinePreview'
import {type FixReceivedMode, useFixApply} from '@/hooks/useFixApply'
import {useReferencePhase} from '@/hooks/useReferencePhase'
import {useReferenceDerivation} from '@/hooks/useReferenceDerivation'
import {useFixReference} from '@/stores/fixReference'
import {useSelectionStore} from '@/stores/selection'
import {type DateSuggestionKey, dateSuggestions} from '@/lib/dateSuggestions'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import {ReceivedModeToggle} from './ReceivedModeToggle'
import {ApplyControls} from './ApplyControls'
import {CancelReferencesButton, NearbyReferenceSort, PickReferencesButton} from './fixShared'
import type {PictureDetail} from '@/lib/types'

const CHIP_ICON: Record<DateSuggestionKey, typeof FileText> = {
    filename: FileText,
    'filename-alt': FileText,
    source: FileClock,
    uploaded: Upload,
}

export function DateFixPanel({target, allowExifEdit}: { target: PictureDetail; allowExifEdit: boolean }) {
    const owned = target.owner_username == null
    const refActive = useFixReference((s) => s.active && s.field === 'date')
    const {begin, exit} = useReferencePhase()
    const {applyOne, invalidate} = useFixApply()
    const queueLand = useSelectionStore((s) => s.queueLand)
    const refDeriv = useReferenceDerivation('date', null)

    const [draft, setDraft] = useState<string | null>(target.captured_at)
    const [saving, setSaving] = useState(false)
    const [receivedMode, setReceivedMode] = useState<FixReceivedMode>('local')
    const manual = useRef(false)

    // Reset on picture change or reference-mode toggle: seed from the derived (mean) reference date in
    // reference mode, else the picture's own date. Render-time (the codebase idiom); seeding here — not
    // only in the async effect — is what makes switching between two targets sharing the same reference
    // date still populate.
    const seedDate = refActive ? refDeriv.dateValue : null
    const resetSig = `${target.id}:${target.captured_at}:${refActive}`
    const lastResetSig = useRef(resetSig)
    if (resetSig !== lastResetSig.current) {
        lastResetSig.current = resetSig
        manual.current = false
        setDraft(seedDate ?? target.captured_at)
    }

    // Re-seed when the derived reference date arrives asynchronously (fetch), unless the user edited it.
    useEffect(() => {
        if (refActive && !manual.current && refDeriv.dateValue) setDraft(refDeriv.dateValue)
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [refActive, refDeriv.dateValue])

    const chips = dateSuggestions(target)
    const parsed = draft ? parseNaive(draft) : null
    const time = parsed?.time ?? '12:00'

    // Queue where to land, then (in reference mode) restore the pre-reference view. `destSig` ties the
    // intent to that restored view so PhotoGrid resolves it only once the view is back on screen (§8).
    const finish = (advance: boolean) => {
        queueLand({anchorId: target.id, advance, destSig: useFixReference.getState().entrySig})
        if (refActive) exit()
    }

    const skip = () => finish(true)
    const cancelReferences = () => {
        queueLand({anchorId: target.id, advance: false, destSig: useFixReference.getState().entrySig})
        exit()
    }

    const apply = async (advance: boolean) => {
        if (!draft) return
        setSaving(true)
        try {
            await applyOne(target.id, owned, {captured_at: draft}, receivedMode)
            invalidate()
            toast.success('Date applied')
            finish(advance)
        } catch (e) {
            toast.error('Could not apply date', {description: apiErrorMessage(e)})
        } finally {
            setSaving(false)
        }
    }

    return (
        <>
            {/* Full-bleed, centered calendar; container shares the calendar's own `bg-background` so the
                two read as one surface (no muted rectangle behind it). */}
            <div className="-mx-3 flex justify-center overflow-hidden border-y border-border bg-background py-2">
                <Calendar
                    mode="single"
                    weekStartsOn={1}
                    selected={parsed?.date}
                    onSelect={(d) => {
                        manual.current = true
                        setDraft(d ? buildNaive(d, time) : null)
                    }}
                    captionLayout="dropdown"
                    className="max-w-full [--cell-size:1.75rem]"
                    classNames={{weekdays: 'flex gap-0.5', week: 'mt-0.5 flex w-full gap-0.5'}}
                />
            </div>

            <div className="flex items-center gap-2">
                <Input
                    type="time"
                    value={time}
                    onChange={(e) => {
                        manual.current = true
                        setDraft(buildNaive(parsed?.date ?? new Date(), e.target.value))
                    }}
                    className="h-8 w-28 text-xs"
                />
                <span className="flex-1 truncate text-right text-xs text-muted-foreground">{draft ? formatNaive(draft) : 'No date yet'}</span>
            </div>

            {refActive ? (
                <div className="space-y-2">
                    <div className="flex items-center gap-2 rounded-md border border-primary/40 bg-primary/5 p-2 text-xs">
                        <Users className="h-4 w-4 shrink-0 text-primary"/>
                        <span className="flex-1">
                            {refDeriv.count === 0
                                ? 'Tap same-time photos in the grid to use as references.'
                                : `${refDeriv.count} reference${refDeriv.count > 1 ? 's' : ''}: the average date is used.`}
                        </span>
                        {refDeriv.loading && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground"/>}
                    </div>
                    {refDeriv.dateMs != null && refDeriv.refTimes.length > 0 && (
                        <div className="rounded-md border border-border">
                            <DateTimelinePreview refTimes={refDeriv.refTimes} derived={refDeriv.dateMs}/>
                        </div>
                    )}
                    <NearbyReferenceSort field="date" target={target}/>
                </div>
            ) : chips.length > 0 ? (
                <div className="space-y-1">
                    <p className="text-[10px] uppercase tracking-wide text-muted-foreground">Suggestions</p>
                    <div className="flex flex-wrap gap-1">
                        {chips.map((c) => {
                            const Icon = CHIP_ICON[c.key]
                            return (
                                <button
                                    key={c.key}
                                    type="button"
                                    onClick={() => {
                                        manual.current = true;
                                        setDraft(c.value)
                                    }}
                                    title={formatNaive(c.value)}
                                    className={cn(
                                        'flex max-w-full items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] transition-colors hover:border-primary/60 hover:text-primary',
                                        draft === c.value ? 'border-primary/60 bg-primary/10 text-primary' : 'border-border text-muted-foreground',
                                    )}
                                >
                                    <Icon className={cn('h-3 w-3 shrink-0', c.lowConfidence && 'text-amber-500')}/>
                                    <span className="truncate">{c.label}: {formatNaive(c.value)}</span>
                                </button>
                            )
                        })}
                    </div>
                </div>
            ) : null}

            {!owned && <ReceivedModeToggle value={receivedMode} onChange={setReceivedMode} allowPropose={allowExifEdit}/>}

            <ApplyControls onApply={() => apply(false)} onApplyNext={() => apply(true)} onSkip={skip} disabled={!draft} saving={saving}/>
            {refActive
                ? <CancelReferencesButton onClick={cancelReferences}/>
                : <PickReferencesButton onClick={() => begin('date', [target.id])}/>}
        </>
    )
}
