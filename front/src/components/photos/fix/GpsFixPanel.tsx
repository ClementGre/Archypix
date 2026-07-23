// GPS-fix surface (feature 30 §5) for a single target, shown as a non-collapsible pane in the details
// panel. A full-bleed map (draggable proposed pin) sits on top and stays for manual tweaking; below it
// the before/after interpolation anchors — or, while picking references, the reference summary in
// their place. Received targets choose local vs propose.

import {useEffect, useMemo, useRef, useState} from 'react'
import {AlertTriangle, Loader2, MapPin, Users} from 'lucide-react'
import {toast} from 'sonner'
import {MapView} from '@/components/common/MapView'
import {OrientedContainImage} from '@/components/photos/OrientedImage'
import {useFixAnchors} from '@/hooks/useFixAnchors'
import {type FixReceivedMode, useFixApply} from '@/hooks/useFixApply'
import {useReferencePhase} from '@/hooks/useReferencePhase'
import {useReferenceDerivation} from '@/hooks/useReferenceDerivation'
import {useFixReference} from '@/stores/fixReference'
import {useFixHighlight} from '@/stores/fixHighlight'
import {useSelectionStore} from '@/stores/selection'
import {formatDistance, formatLatLng, haversineM, naiveToMs} from '@/lib/gpsInterpolation'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import {ReceivedModeToggle} from './ReceivedModeToggle'
import {ApplyControls} from './ApplyControls'
import {CancelReferencesButton, NearbyReferenceSort, PickReferencesButton} from './fixShared'
import type {PictureDetail} from '@/lib/types'

const DAY_MS = 24 * 60 * 60 * 1000

/** |a − b| as a compact label (s / min / h / d), or null when either is missing. */
function gapLabel(a: string | null | undefined, b: string | null | undefined): { text: string; ms: number } | null {
    const ma = naiveToMs(a), mb = naiveToMs(b)
    if (ma == null || mb == null) return null
    const ms = Math.abs(mb - ma)
    const s = Math.round(ms / 1000)
    if (s < 60) return {text: `${s}s`, ms}
    const m = Math.round(s / 60)
    if (m < 60) return {text: `${m} min`, ms}
    const h = Math.round(m / 60)
    if (h < 48) return {text: `${h} h`, ms}
    return {text: `${Math.round(h / 24)} d`, ms}
}

function AnchorThumb({label, anchor, loading, gap}: {
    label: string
    anchor: { thumbnail_url: string | null; orientation: number | null } | null
    loading: boolean
    gap: { text: string; ms: number } | null
}) {
    // No anchor on this side: while still fetching show a spinner, otherwise say so plainly rather
    // than leaving an empty box (there is simply no earlier/later GPS-bearing photo to interpolate).
    if (!anchor) {
        return (
            <div className="flex h-14 flex-1 flex-col items-center justify-center gap-1 text-center">
                <span className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</span>
                {loading
                    ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground"/>
                    : <span className="text-[11px] text-muted-foreground/70">No {label.toLowerCase()} photo</span>}
            </div>
        )
    }
    const far = gap != null && gap.ms >= DAY_MS
    return (
        <div className="flex flex-1 flex-col items-center gap-1">
            <span className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</span>
            <div className="relative h-14 w-14 overflow-hidden rounded bg-checkerboard ring-1 ring-sky-400/60">
                {anchor.thumbnail_url &&
                    <OrientedContainImage src={anchor.thumbnail_url} alt="" orientation={anchor.orientation} className="h-14 w-14"/>}
            </div>
            {gap && (
                <span className={cn('flex items-center gap-0.5 text-[10px]', far ? 'text-amber-500' : 'text-muted-foreground')}>
                    {far && <AlertTriangle className="h-2.5 w-2.5"/>}{gap.text}
                </span>
            )}
        </div>
    )
}

export function GpsFixPanel({target, allowExifEdit}: { target: PictureDetail; allowExifEdit: boolean }) {
    const owned = target.owner_username == null
    const refActive = useFixReference((s) => s.active && s.field === 'gps')
    const {begin, exit} = useReferencePhase()
    const {applyOne, invalidate} = useFixApply()
    const setAnchorIds = useFixHighlight((s) => s.setAnchorIds)
    const queueLand = useSelectionStore((s) => s.queueLand)

    const {before, after, proposed, loading: anchorsLoading, undatedTarget} = useFixAnchors(target)
    const refDeriv = useReferenceDerivation('gps', target.captured_at)
    const seed = refActive ? refDeriv.gpsValue : proposed

    const [draft, setDraft] = useState<{ lat: number | null; lng: number | null; alt: number | null }>({
        lat: target.gps_lat, lng: target.gps_lng, alt: target.gps_alt,
    })
    const [saving, setSaving] = useState(false)
    const [receivedMode, setReceivedMode] = useState<FixReceivedMode>('local')
    const manual = useRef(false)

    // Reset on target change or reference-mode toggle: seed from the current suggestion (the reference
    // derivation in reference mode, else the interpolation), falling back to the picture's own GPS.
    // Render-time (the codebase idiom); seeding here — not only in the async effect below — is what
    // makes switching between two targets that share the same reference-derived point still populate.
    const resetSig = `${target.id}:${target.gps_lat}:${target.gps_lng}:${target.gps_alt}:${refActive}`
    const lastResetSig = useRef(resetSig)
    if (resetSig !== lastResetSig.current) {
        lastResetSig.current = resetSig
        manual.current = false
        setDraft(seed ? {lat: seed.lat, lng: seed.lng, alt: seed.alt} : {lat: target.gps_lat, lng: target.gps_lng, alt: target.gps_alt})
    }

    // Re-seed when the suggestion arrives asynchronously (fetch) after mount, unless the user moved the pin.
    useEffect(() => {
        if (!manual.current && seed) setDraft({lat: seed.lat, lng: seed.lng, alt: seed.alt})
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [seed?.lat, seed?.lng])

    // Highlight the before/after anchors in the grid — but not while picking references (the reference
    // set is highlighted through the selection instead).
    useEffect(() => {
        setAnchorIds(refActive ? [] : [before?.id, after?.id].filter((x): x is string => !!x))
        return () => setAnchorIds([])
    }, [refActive, before?.id, after?.id, setAnchorIds])

    const extraMarkers = useMemo(() => {
        const pts = refActive ? refDeriv.refAnchors : [before, after].filter((a): a is NonNullable<typeof a> => !!a)
        return pts.map((a) => ({lat: a.lat, lng: a.lng, color: '#0ea5e9'}))
    }, [refActive, refDeriv.refAnchors, before, after])

    const hasValue = draft.lat != null && draft.lng != null

    // Queue where to land, then (in reference mode) restore the pre-reference view. `destSig` ties the
    // intent to that restored view so PhotoGrid resolves it only once the view is actually back on
    // screen (the restore navigation lands a render after the store exits reference mode). `advance`
    // selects the next still-missing picture after the target, otherwise the target stays selected (§8).
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
        if (!hasValue) return
        setSaving(true)
        try {
            await applyOne(target.id, owned, {gps_lat: draft.lat!, gps_lng: draft.lng!, gps_alt: draft.alt}, receivedMode)
            invalidate()
            toast.success('Location applied')
            finish(advance)
        } catch (e) {
            toast.error('Could not apply location', {description: apiErrorMessage(e)})
        } finally {
            setSaving(false)
        }
    }

    // Undated target with no references yet: interpolation needs a time anchor (§12.1).
    if (undatedTarget && !refActive) {
        return (
            <>
                <div className="flex items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2 text-amber-600 dark:text-amber-400">
                    <AlertTriangle className="h-4 w-4 shrink-0"/>
                    <span className="text-xs">No capture date, so GPS can&apos;t be interpolated. Fix the date first, or pick references.</span>
                </div>
                <PickReferencesButton onClick={() => begin('gps', [target.id])}/>
            </>
        )
    }

    const beforeGap = gapLabel(before?.time, target.captured_at)
    const afterGap = gapLabel(after?.time, target.captured_at)
    const anchorM = before && after ? haversineM(before.lat, before.lng, after.lat, after.lng) : null
    const uncertain = (anchorM != null && anchorM > 10_000)

    return (
        <>
            {/* Full-bleed map (no sidebar margin), border kept so favourites stay contained. */}
            <div className="-mx-3 overflow-hidden border-y border-border">
                <MapView
                    mode="point"
                    point={{lat: draft.lat, lng: draft.lng}}
                    onPoint={(lat, lng) => {
                        manual.current = true
                        setDraft((d) => ({...d, lat, lng}))
                    }}
                    extraMarkers={extraMarkers}
                    className="h-52 w-full"
                    expandable
                />
            </div>

            {refActive ? (
                <div className="space-y-2">
                    <div className="flex items-center gap-2 rounded-md border border-primary/40 bg-primary/5 p-2 text-xs">
                        <Users className="h-4 w-4 shrink-0 text-primary"/>
                        <span className="flex-1">
                            {refDeriv.count === 0
                                ? 'Tap same-place photos in the grid to use as references.'
                                : `${refDeriv.count} reference${refDeriv.count > 1 ? 's' : ''}: location is ${refDeriv.count > 1 ? 'averaged' : 'copied'} here.`}
                        </span>
                        {refDeriv.loading && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground"/>}
                    </div>
                    <NearbyReferenceSort field="gps" target={target}/>
                </div>
            ) : (
                <div className="flex items-start gap-2">
                    <AnchorThumb label="Before" anchor={before} loading={anchorsLoading} gap={beforeGap}/>
                    <div className="flex flex-col items-center pt-5">
                        {anchorsLoading ? <Loader2 className="h-4 w-4 animate-spin text-muted-foreground"/> :
                            <MapPin className={cn('h-4 w-4', hasValue ? 'text-primary' : 'text-muted-foreground')}/>}
                    </div>
                    <AnchorThumb label="After" anchor={after} loading={anchorsLoading} gap={afterGap}/>
                </div>
            )}

            <div className="text-center">
                {!refActive && anchorM != null && (
                    <p
                        className={cn('flex items-center justify-center gap-1 text-xs', uncertain ? 'text-amber-500' : 'text-muted-foreground')}
                        title={uncertain ? 'The two photos are far apart in time or space, so the interpolated point is only a rough guess' : undefined}
                    >
                        {uncertain && <AlertTriangle className="h-3 w-3"/>}
                        {formatDistance(anchorM)} apart
                    </p>
                )}
                <p className="text-xs text-muted-foreground">
                    {hasValue ? formatLatLng(draft.lat!, draft.lng!) : 'No location yet. Drag the pin or pick references.'}
                </p>
            </div>

            {!owned && <ReceivedModeToggle value={receivedMode} onChange={setReceivedMode} allowPropose={allowExifEdit}/>}

            <ApplyControls onApply={() => apply(false)} onApplyNext={() => apply(true)} onSkip={skip} disabled={!hasValue} saving={saving}/>
            {refActive
                ? <CancelReferencesButton onClick={cancelReferences}/>
                : <PickReferencesButton onClick={() => begin('gps', [target.id])}/>}
        </>
    )
}
