// Small shared pieces for the fix panels (feature 30): a consistent "Pick references" button (same
// wording across GPS / Date / batch), a "Cancel references" control, and the non-collapsible pane
// wrapper whose fixed frame justifies the full-bleed map / calendar inside.

import type {ReactNode} from 'react'
import {CalendarClock, MapPin, Users, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import type {FixMode, PictureDetail} from '@/lib/types'

/** Consistent references entry button. Label is identical everywhere so it reads the same. */
export function PickReferencesButton({onClick}: { onClick: () => void }) {
    return (
        <Button
            variant="outline"
            size="sm"
            onClick={onClick}
            className="h-auto w-full justify-start gap-1.5 whitespace-normal py-1.5 text-left text-xs text-muted-foreground"
        >
            <Users className="h-3.5 w-3.5 shrink-0"/> Pick references from other photos
        </Button>
    )
}

/** Leave reference-picking and return to the normal fix view. */
export function CancelReferencesButton({onClick}: { onClick: () => void }) {
    return (
        <Button variant="ghost" size="sm" onClick={onClick} className="h-auto w-full justify-center gap-1.5 py-1 text-xs text-muted-foreground">
            <X className="h-3.5 w-3.5 shrink-0"/> Cancel reference picking
        </Button>
    )
}

/**
 * A one-click proximity sort of the grid to surface likely references (feature 30 §7), restored on
 * phase exit: GPS targets sort by *time* proximity (same time is usually same place), date targets by
 * *place* proximity (same place is usually same trip, and the geo sort adds the distance badges).
 * Renders nothing when the target lacks the needed field.
 */
export function NearbyReferenceSort({field, target}: { field: FixMode; target: PictureDetail }) {
    const {params, update} = useGalleryParams()
    const nearbyTime = field === 'gps' ? target.captured_at : null
    const nearbyPlace = field === 'date' && target.gps_lat != null && target.gps_lng != null
        ? {lat: target.gps_lat, lng: target.gps_lng}
        : null
    if (!nearbyTime && !nearbyPlace) return null

    const active =
        (nearbyTime != null && params.sort === 'time_near' && params.nearTime === nearbyTime) ||
        (nearbyPlace != null && params.sort === 'geo_near' && params.nearLat === nearbyPlace.lat && params.nearLng === nearbyPlace.lng)
    const clear = () => update({sort: 'captured_at', order: 'desc', nearTime: null, nearLat: null, nearLng: null})
    const apply = () =>
        nearbyTime != null
            ? update({sort: 'time_near', nearTime: nearbyTime, order: 'asc'})
            : nearbyPlace && update({sort: 'geo_near', nearLat: nearbyPlace.lat, nearLng: nearbyPlace.lng})

    return (
        <Button
            variant={active ? 'secondary' : 'outline'}
            size="sm"
            onClick={active ? clear : apply}
            className="h-auto w-full justify-start gap-1.5 whitespace-normal py-1.5 text-left text-xs"
        >
            {field === 'gps' ? <CalendarClock className="h-3.5 w-3.5 shrink-0"/> : <MapPin className="h-3.5 w-3.5 shrink-0"/>}
            {active
                ? `Sorted by ${field === 'gps' ? 'time' : 'place'} near this photo`
                : field === 'gps'
                    ? 'Show photos nearby in time'
                    : 'Show photos nearby in place'}
        </Button>
    )
}

/**
 * A non-collapsible pane in the details sidebar. Unlike the foldable `Section`, its frame is always
 * present, which is why the full-bleed map / calendar inside it reads correctly.
 */
export function FixPane({title, children}: { title: string; children: ReactNode }) {
    return (
        <div className="border-b border-border">
            <div className="py-2 text-sm font-medium">{title}</div>
            <div className="space-y-3 pb-3">{children}</div>
        </div>
    )
}
