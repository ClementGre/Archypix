import type {ReactNode} from 'react'
import {useEffect, useRef, useState} from 'react'
import {Navigation, X} from 'lucide-react'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Button} from '@/components/ui/button'
import {NumberInput} from '@/components/ui/number-input'
import {Label} from '@/components/ui/label'

// ── Vanilla Leaflet, loaded once from CDN ──────────────────────────────────────
// We avoid the react-leaflet wrapper (which pulled a duplicate React copy and
// crashed with "Invalid hook call"). Plain Leaflet has no React dependency.

interface LMap {
    setView(center: [number, number], zoom?: number): LMap

    on(event: string, cb: (e: { latlng: { lat: number; lng: number } }) => void): void

    remove(): void

    invalidateSize(): void
}

interface LMarker {
    setLatLng(center: [number, number]): LMarker

    addTo(map: LMap): LMarker
}

interface LTileLayer {
    addTo(map: LMap): LTileLayer
}

interface LeafletStatic {
    map(el: HTMLElement, opts?: Record<string, unknown>): LMap

    tileLayer(url: string, opts?: Record<string, unknown>): LTileLayer

    marker(center: [number, number], opts?: Record<string, unknown>): LMarker
}

declare global {
    interface Window {
        L?: LeafletStatic
    }
}

let leafletPromise: Promise<LeafletStatic> | null = null

function loadLeaflet(): Promise<LeafletStatic> {
    if (window.L) return Promise.resolve(window.L)
    if (leafletPromise) return leafletPromise
    leafletPromise = new Promise<LeafletStatic>((resolve, reject) => {
        const css = document.createElement('link')
        css.rel = 'stylesheet'
        css.href = 'https://unpkg.com/leaflet@1.9.4/dist/leaflet.css'
        document.head.appendChild(css)

        const script = document.createElement('script')
        script.src = 'https://unpkg.com/leaflet@1.9.4/dist/leaflet.js'
        script.async = true
        script.onload = () => (window.L ? resolve(window.L) : reject(new Error('Leaflet failed to load')))
        script.onerror = () => reject(new Error('Leaflet failed to load'))
        document.head.appendChild(script)
    })
    return leafletPromise
}

// ── Map sub-component ──────────────────────────────────────────────────────────

function LeafletMap({
                        lat,
                        lng,
                        onPick,
                    }: {
    lat: number | null
    lng: number | null
    onPick: (lat: number, lng: number) => void
}) {
    const containerRef = useRef<HTMLDivElement>(null)
    const mapRef = useRef<LMap | null>(null)
    const markerRef = useRef<LMarker | null>(null)
    const onPickRef = useRef(onPick)
    onPickRef.current = onPick

    // Init once.
    useEffect(() => {
        let cancelled = false
        loadLeaflet().then((L) => {
            if (cancelled || !containerRef.current || mapRef.current) return
            const center: [number, number] = lat != null && lng != null ? [lat, lng] : [20, 0]
            const zoom = lat != null && lng != null ? 13 : 2
            const map = L.map(containerRef.current, {zoomControl: true, attributionControl: false})
            map.setView(center, zoom)
            L.tileLayer('https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png', {
                maxZoom: 19,
            }).addTo(map)
            map.on('click', (e) => onPickRef.current(e.latlng.lat, e.latlng.lng))
            mapRef.current = map
            if (lat != null && lng != null) {
                markerRef.current = L.marker([lat, lng]).addTo(map)
            }
            // Popover content animates in; recompute size once settled.
            setTimeout(() => map.invalidateSize(), 60)
        })
        return () => {
            cancelled = true
            mapRef.current?.remove()
            mapRef.current = null
            markerRef.current = null
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [])

    // Reflect external coordinate changes (manual input / current location).
    useEffect(() => {
        const map = mapRef.current
        if (!map || lat == null || lng == null) return
        const L = window.L
        if (!L) return
        if (markerRef.current) {
            markerRef.current.setLatLng([lat, lng])
        } else {
            markerRef.current = L.marker([lat, lng]).addTo(map)
        }
        map.setView([lat, lng])
    }, [lat, lng])

    return <div ref={containerRef} className="h-44 w-full"/>
}

// ── Popover ────────────────────────────────────────────────────────────────────

interface GpsValue {
    lat: string
    lng: string
    alt: string
}

interface GpsPickerPopoverProps {
    value: GpsValue
    onChange: (value: GpsValue) => void
    children: ReactNode
}

export function GpsPickerPopover({value, onChange, children}: GpsPickerPopoverProps) {
    const [open, setOpen] = useState(false)
    const [locating, setLocating] = useState(false)

    const lat = value.lat !== '' && !isNaN(parseFloat(value.lat)) ? parseFloat(value.lat) : null
    const lng = value.lng !== '' && !isNaN(parseFloat(value.lng)) ? parseFloat(value.lng) : null

    function handleCurrentLocation() {
        if (!navigator.geolocation) return
        setLocating(true)
        navigator.geolocation.getCurrentPosition(
            (pos) => {
                onChange({
                    lat: pos.coords.latitude.toFixed(6),
                    lng: pos.coords.longitude.toFixed(6),
                    alt: pos.coords.altitude != null ? String(Math.round(pos.coords.altitude)) : value.alt,
                })
                setLocating(false)
            },
            () => setLocating(false),
        )
    }

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>{children}</PopoverTrigger>
            <PopoverContent className="w-80 space-y-3 p-3" side="left" align="start">
                <div className="flex items-center justify-between">
                    <p className="text-sm font-medium">GPS location</p>
                    <div className="flex items-center gap-1">
                        <Button
                            variant="outline"
                            size="sm"
                            className="h-7 gap-1 text-xs"
                            onClick={handleCurrentLocation}
                            disabled={locating}
                        >
                            <Navigation className="h-3 w-3"/>
                            {locating ? 'Locating…' : 'Current location'}
                        </Button>
                        <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 gap-1 text-xs"
                            onClick={() => {
                                onChange({lat: '', lng: '', alt: ''})
                                setOpen(false)
                            }}
                        >
                            <X className="h-3 w-3"/>
                            Clear
                        </Button>
                    </div>
                </div>

                {open && (
                    <div className="overflow-hidden rounded-md border border-border">
                        <LeafletMap
                            lat={lat}
                            lng={lng}
                            onPick={(la, ln) =>
                                onChange({...value, lat: la.toFixed(6), lng: ln.toFixed(6)})
                            }
                        />
                    </div>
                )}
                <p className="text-[11px] text-muted-foreground">Click the map to drop a pin.</p>

                <div className="grid grid-cols-2 gap-2">
                    <div className="space-y-1">
                        <Label className="text-xs text-muted-foreground">Latitude</Label>
                        <NumberInput
                            step="any"
                            placeholder="48.8566"
                            value={value.lat}
                            onChange={(e) => onChange({...value, lat: e.target.value})}
                            className="h-8"
                        />
                    </div>
                    <div className="space-y-1">
                        <Label className="text-xs text-muted-foreground">Longitude</Label>
                        <NumberInput
                            step="any"
                            placeholder="2.3522"
                            value={value.lng}
                            onChange={(e) => onChange({...value, lng: e.target.value})}
                            className="h-8"
                        />
                    </div>
                </div>
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Altitude (m)</Label>
                    <NumberInput
                        step="1"
                        placeholder="35"
                        value={value.alt}
                        onChange={(e) => onChange({...value, alt: e.target.value})}
                        className="h-8"
                    />
                </div>
            </PopoverContent>
        </Popover>
    )
}
