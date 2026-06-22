// GPS-zone picker for rule predicates: one trigger button opens a popover where the user toggles
// between a rectangle and a circle and moves/resizes it on a real map (shared `MapView`), with
// numeric inputs kept in sync. Emits a `gps_bbox` or `gps_radius` zone.

import {useState} from 'react'
import {MapPin} from 'lucide-react'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {type Bbox, MapView} from '@/components/common/MapView'
import type {GpsBbox, GpsRadius} from '@/lib/types'

export type Zone = { kind: 'bbox'; box: GpsBbox } | { kind: 'circle'; radius: GpsRadius }

interface MapZonePopoverProps {
    zone: Zone
    onChange: (z: Zone) => void
}

const toBbox = (b: GpsBbox): Bbox => ({latMin: b.lat_min, latMax: b.lat_max, lonMin: b.lon_min, lonMax: b.lon_max})
const fromBbox = (b: Bbox): GpsBbox => ({lat_min: b.latMin, lat_max: b.latMax, lon_min: b.lonMin, lon_max: b.lonMax})

/** Convert a box to an enclosing-ish circle (centre + half-diagonal-ish radius). */
function boxToCircle(b: GpsBbox): GpsRadius {
    const lat = (b.lat_min + b.lat_max) / 2
    const lng = (b.lon_min + b.lon_max) / 2
    const dLatKm = ((b.lat_max - b.lat_min) / 2) * 111.32
    const dLonKm = ((b.lon_max - b.lon_min) / 2) * 111.32 * Math.cos((lat * Math.PI) / 180)
    const km = Math.max(0.5, Math.round(Math.hypot(dLatKm, dLonKm) * 100) / 100)
    return {lat: round6(lat), lng: round6(lng), km}
}

/** Convert a circle to a bounding box that contains it. */
function circleToBox(c: GpsRadius): GpsBbox {
    const dLat = c.km / 111.32
    const dLon = c.km / (111.32 * Math.cos((c.lat * Math.PI) / 180) || 1)
    return {
        lat_min: round4(c.lat - dLat),
        lat_max: round4(c.lat + dLat),
        lon_min: round4(c.lng - dLon),
        lon_max: round4(c.lng + dLon),
    }
}

const round4 = (n: number) => Math.round(n * 1e4) / 1e4
const round6 = (n: number) => Math.round(n * 1e6) / 1e6

function summary(zone: Zone): string {
    if (zone.kind === 'bbox') {
        const b = zone.box
        return `box [${b.lat_min}, ${b.lat_max}] × [${b.lon_min}, ${b.lon_max}]`
    }
    return `within ${zone.radius.km} km of ${zone.radius.lat}, ${zone.radius.lng}`
}

export function MapZonePopover({zone, onChange}: MapZonePopoverProps) {
    const [open, setOpen] = useState(false)

    const setShape = (kind: 'bbox' | 'circle') => {
        if (kind === zone.kind) return
        if (kind === 'circle') onChange({kind: 'circle', radius: boxToCircle((zone as { box: GpsBbox }).box)})
        else onChange({kind: 'bbox', box: circleToBox((zone as { radius: GpsRadius }).radius)})
    }

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>
                <Button variant="outline" size="sm" className="h-7 gap-1.5 text-xs font-normal">
                    <MapPin className="h-3.5 w-3.5 text-muted-foreground"/>
                    <span className="max-w-[16rem] truncate">{summary(zone)}</span>
                </Button>
            </PopoverTrigger>
            <PopoverContent className="w-[28rem] space-y-3 p-3" align="start">
                <div className="inline-flex overflow-hidden rounded-md border text-xs">
                    {(['bbox', 'circle'] as const).map((k) => (
                        <button
                            key={k}
                            type="button"
                            onClick={() => setShape(k)}
                            className={`px-3 py-1 font-medium transition-colors ${
                                zone.kind === k ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-accent'
                            }`}
                        >
                            {k === 'bbox' ? 'Rectangle' : 'Circle'}
                        </button>
                    ))}
                </div>

                <div className="overflow-hidden rounded-md border border-border">
                    {zone.kind === 'bbox' ? (
                        <MapView
                            mode="bbox"
                            bbox={toBbox(zone.box)}
                            onBbox={(b) => onChange({kind: 'bbox', box: fromBbox(b)})}
                            className="h-72 w-full"
                        />
                    ) : (
                        <MapView
                            mode="circle"
                            circle={zone.radius}
                            onCircle={(c) => onChange({kind: 'circle', radius: c})}
                            className="h-72 w-full"
                        />
                    )}
                </div>
                <p className="text-[11px] text-muted-foreground">
                    {zone.kind === 'bbox'
                        ? 'Drag the green corners to resize, the blue dot to move.'
                        : 'Drag the blue dot to move, the green dot to set the radius.'}
                </p>

                {zone.kind === 'bbox' ? (
                    <div className="grid grid-cols-2 gap-2">
                        {(['lat_min', 'lat_max', 'lon_min', 'lon_max'] as const).map((k) => (
                            <div key={k} className="space-y-1">
                                <Label className="text-xs text-muted-foreground">{k}</Label>
                                <Input
                                    type="number"
                                    step="any"
                                    value={zone.box[k]}
                                    onChange={(e) =>
                                        onChange({kind: 'bbox', box: {...zone.box, [k]: Number(e.target.value)}})
                                    }
                                    className="h-8 text-xs"
                                />
                            </div>
                        ))}
                    </div>
                ) : (
                    <div className="grid grid-cols-3 gap-2">
                        {(['lat', 'lng', 'km'] as const).map((k) => (
                            <div key={k} className="space-y-1">
                                <Label className="text-xs text-muted-foreground">{k}</Label>
                                <Input
                                    type="number"
                                    step="any"
                                    value={zone.radius[k]}
                                    onChange={(e) =>
                                        onChange({kind: 'circle', radius: {...zone.radius, [k]: Number(e.target.value)}})
                                    }
                                    className="h-8 text-xs"
                                />
                            </div>
                        ))}
                    </div>
                )}
            </PopoverContent>
        </Popover>
    )
}
