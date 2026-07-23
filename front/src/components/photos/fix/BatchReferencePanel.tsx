// Batch reference-picking pane (feature 30 §7/§8): shown in the details panel while picking references
// for a multi-selection. Previews the one derived value (references' centroid / mean) on a map or
// timeline and applies it to all targets through the bulk preview, restoring the filters on exit.

import {useState} from 'react'
import {Loader2, Users} from 'lucide-react'
import {useFixReference} from '@/stores/fixReference'
import {useReferencePhase} from '@/hooks/useReferencePhase'
import {useReferenceDerivation} from '@/hooks/useReferenceDerivation'
import {useGridItems} from '@/stores/gridItems'
import {Button} from '@/components/ui/button'
import {MapView} from '@/components/common/MapView'
import {formatNaive} from '@/components/photos/detail/DateTimePickerPopover'
import {CancelReferencesButton, FixPane} from './fixShared'
import {DateTimelinePreview} from './DateTimelinePreview'
import {FixBulkDialog} from './FixBulkDialog'
import {referenceBulkRows} from '@/lib/fixBulk'
import type {FixValue} from '@/hooks/useFixApply'
import type {PictureListItem} from '@/lib/types'

export function BatchReferencePanel() {
    const {field, targetIds} = useFixReference()
    const {exit} = useReferencePhase()
    const grid = useGridItems((s) => s.items)
    const [showDialog, setShowDialog] = useState(false)
    const deriv = useReferenceDerivation(field ?? 'gps', null)

    // A single target lets a bracket interpolate; batch uses the centroid / mean (null target time).
    const value: FixValue | null =
        field === 'gps'
            ? deriv.gpsValue ? {gps_lat: deriv.gpsValue.lat, gps_lng: deriv.gpsValue.lng, gps_alt: deriv.gpsValue.alt} : null
            : deriv.dateValue ? {captured_at: deriv.dateValue} : null

    const targetItems = grid.filter((i) => targetIds.includes(i.id)) as PictureListItem[]
    const hasReceived = targetItems.some((i) => !i.owned)

    return (
        <FixPane title="References">
            <div className="flex items-center gap-2 rounded-md border border-primary/40 bg-primary/5 p-2 text-xs">
                <Users className="h-4 w-4 shrink-0 text-primary"/>
                <span className="flex-1">
                    {deriv.count === 0
                        ? `Tap the ${field === 'gps' ? 'same-place' : 'same-time'} photos to use for ${targetIds.length} targets.`
                        : `${deriv.count} reference${deriv.count > 1 ? 's' : ''} applied to ${targetIds.length} targets.`}
                </span>
                {deriv.loading && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground"/>}
            </div>

            {field === 'gps' && deriv.gpsValue && (
                <div className="-mx-3 overflow-hidden border-y border-border">
                    <MapView
                        mode="point"
                        interactive={false}
                        point={{lat: deriv.gpsValue.lat, lng: deriv.gpsValue.lng}}
                        extraMarkers={deriv.refAnchors.map((a) => ({lat: a.lat, lng: a.lng, color: '#0ea5e9'}))}
                        className="h-40 w-full"
                    />
                </div>
            )}
            {field === 'date' && deriv.dateMs != null && deriv.refTimes.length > 0 && (
                <div className="rounded-md border border-border">
                    <DateTimelinePreview refTimes={deriv.refTimes} derived={deriv.dateMs}/>
                    <p className="border-t border-border px-2.5 py-1.5 text-xs">
                        <span className="text-muted-foreground">Average date: </span>{formatNaive(deriv.dateValue)}
                    </p>
                </div>
            )}

            <Button size="sm" className="w-full" disabled={deriv.loading || !value} onClick={() => setShowDialog(true)}>
                Preview &amp; apply to {targetIds.length}
            </Button>
            <CancelReferencesButton onClick={() => exit()}/>

            {showDialog && (
                <FixBulkDialog
                    open
                    onOpenChange={(o) => !o && setShowDialog(false)}
                    field={field ?? 'gps'}
                    title={`Apply reference ${field === 'gps' ? 'location' : 'date'} to ${targetIds.length}`}
                    initialRows={referenceBulkRows(targetItems, value)}
                    hasReceived={hasReceived}
                    onApplied={() => exit()}
                />
            )}
        </FixPane>
    )
}
