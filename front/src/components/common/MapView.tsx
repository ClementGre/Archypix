// A reusable, imperatively-driven Leaflet map. Three modes:
//   • point  — a draggable pin (EXIF GPS picking; click the map to place/move it)
//   • bbox    — a rectangle with draggable SW/NE corner handles + a centre move handle
//   • circle  — a circle with a draggable centre + an edge handle that sets the radius (km)
// Factored so both the EXIF point picker and the rule GPS-zone picker share one map.
//
// The basemap is user-selectable (Streets / Satellite / OSM / Light / Dark — persisted globally),
// there's a fullscreen (enlarge) toggle, plus a "Save here" / "My location" control and saved
// favourite locations (localStorage) shown as star pins you can click to centre on.

import {useEffect, useRef, useState} from 'react'
import {Check, Crosshair, LocateFixed, Maximize2, Minus, Pencil, Plus, Star, Trash2, X} from 'lucide-react'
import {basemapById, BASEMAPS, type LatLng, type LCircle, type LLayer, type LMap, type LMarker, loadLeaflet, type LRectangle,} from '@/lib/leaflet'
import {Dialog, DialogContent} from '@/components/ui/dialog'
import {useMapStyle} from '@/stores/mapStyle'
import {coordName, type FavoriteLocation, useFavoriteLocations} from '@/stores/favoriteLocations'

export interface Bbox {
    latMin: number
    latMax: number
    lonMin: number
    lonMax: number
}

export interface CircleZone {
    lat: number
    lng: number
    km: number
}

interface MapViewProps {
    mode: 'point' | 'bbox' | 'circle'
    point?: { lat: number | null; lng: number | null }
    onPoint?: (lat: number, lng: number) => void
    bbox?: Bbox
    onBbox?: (b: Bbox) => void
    circle?: CircleZone
    onCircle?: (c: CircleZone) => void
    className?: string
    /** When true (default), show the enlarge button that opens the map in a large dialog. */
    expandable?: boolean
    /**
     * When false, the map is a read-only display: no draggable handles, no favourites.
     * Used by the batch-aggregate GPS preview.
     */
    interactive?: boolean
    /**
     * Extra static (non-draggable) markers layered on the map — the GPS-fix before/after
     * interpolation anchors (feature 30 §5.1). Default colour is a neutral slate.
     */
    extraMarkers?: { lat: number; lng: number; color?: string; label?: string }[]
}

const HANDLE_HTML = (color: string) =>
    `<div style="width:14px;height:14px;border-radius:9999px;background:${color};border:2px solid white;box-shadow:0 0 0 1px rgba(0,0,0,.4)"></div>`
const FAV_HTML =
    `<div style="font-size:18px;line-height:14px;color:#f59e0b;text-shadow:0 0 2px rgba(0,0,0,.6),0 0 2px rgba(0,0,0,.6)">★</div>`

export function MapView(props: MapViewProps) {
    const {mode, point, onPoint, bbox, onBbox, circle, onCircle, className, expandable = true, interactive = true, extraMarkers} = props
    const containerRef = useRef<HTMLDivElement>(null)
    const mapRef = useRef<LMap | null>(null)
    const tileRef = useRef<LLayer | null>(null)

    const basemap = useMapStyle((s) => s.basemap)
    const setBasemap = useMapStyle((s) => s.setBasemap)
    const favorites = useFavoriteLocations((s) => s.favorites)
    const addFav = useFavoriteLocations((s) => s.add)
    const renameFav = useFavoriteLocations((s) => s.rename)
    const removeFav = useFavoriteLocations((s) => s.remove)

    const [editingFav, setEditingFav] = useState<string | null>(null)
    const [draftName, setDraftName] = useState('')
    const [expanded, setExpanded] = useState(false)

    // Latest callbacks/values/basemap, read inside Leaflet handlers without re-binding them.
    const cb = useRef({onPoint, onBbox, onCircle})
    cb.current = {onPoint, onBbox, onCircle}
    const latest = useRef({point, bbox, circle})
    latest.current = {point, bbox, circle}
    const basemapRef = useRef(basemap)
    basemapRef.current = basemap

    const pin = useRef<LMarker | null>(null)
    const rect = useRef<LRectangle | null>(null)
    const circ = useRef<LCircle | null>(null)
    const handles = useRef<LMarker[]>([])
    const favMarkers = useRef<LMarker[]>([])
    const favoritesRef = useRef<FavoriteLocation[]>(favorites)
    favoritesRef.current = favorites
    const extraLayers = useRef<LMarker[]>([])
    const extraRef = useRef(extraMarkers)
    extraRef.current = extraMarkers

    // (Re)draw the static extra markers (GPS-fix anchors). Safe to call before/after map init.
    const renderExtra = useRef<() => void>(() => {
    })
    renderExtra.current = () => {
        const map = mapRef.current
        const L = window.L
        if (!map || !L) return
        extraLayers.current.forEach((m) => m.remove())
        // Skip non-finite coords: `L.marker([null, …])` yields a marker with a null `_latlng` (Leaflet's
        // toLatLng treats `null` as an object), which then throws on add and on every fitBounds/pan.
        extraLayers.current = (extraRef.current ?? []).filter((e) => Number.isFinite(e.lat) && Number.isFinite(e.lng)).map((e) =>
            L.marker([e.lat, e.lng], {
                icon: L.divIcon({className: '', html: HANDLE_HTML(e.color ?? '#64748b'), iconSize: [14, 14], iconAnchor: [7, 7]}),
                interactive: false,
                // Keep these static markers (GPS-fix reference points) below the draggable pin so the
                // picture's own point is always visible where they overlap.
                zIndexOffset: -1000,
            }).addTo(map),
        )
    }

    // Fit the map so the pin AND all extra markers (GPS-fix anchors / references) are visible.
    const fitPoints = useRef<() => void>(() => {
    })
    fitPoints.current = () => {
        const map = mapRef.current
        if (!map || mode !== 'point') return
        const pts: [number, number][] = []
        const p = latest.current.point
        if (p?.lat != null && p?.lng != null) pts.push([p.lat, p.lng])
        for (const e of extraRef.current ?? []) if (Number.isFinite(e.lat) && Number.isFinite(e.lng)) pts.push([e.lat, e.lng])
        if (pts.length === 0) return
        if (pts.length === 1) {
            map.setView(pts[0], 13)
            return
        }
        const lats = pts.map((q) => q[0])
        const lngs = pts.map((q) => q[1])
        map.fitBounds(
            [[Math.min(...lats), Math.min(...lngs)], [Math.max(...lats), Math.max(...lngs)]],
            {padding: [30, 30], maxZoom: 15},
        )
    }

    // (Re)draw favourite pins from the latest list. Safe to call before/after map init.
    const renderFavorites = useRef<() => void>(() => {
    })
    renderFavorites.current = () => {
        const map = mapRef.current
        const L = window.L
        if (!map || !L) return
        favMarkers.current.forEach((m) => m.remove())
        favMarkers.current = favoritesRef.current.map((f) => {
            const m = L.marker([f.lat, f.lng], {
                icon: L.divIcon({className: '', html: FAV_HTML, iconSize: [18, 18], iconAnchor: [9, 9]}),
            }).addTo(map)
            m.bindTooltip(f.name, {direction: 'top'})
            m.on('click', () => applyRef.current(f.lat, f.lng))
            return m
        })
    }

    // Centre the current shape/pin on a point (used by favourites + "my location").
    const applyRef = useRef<(lat: number, lng: number) => void>(() => {
    })
    applyRef.current = (lat: number, lng: number) => {
        const map = mapRef.current
        if (!map) return
        if (mode === 'point') {
            cb.current.onPoint?.(round(lat), round(lng))
        } else if (mode === 'bbox') {
            const cur = latest.current.bbox!
            const halfLat = (cur.latMax - cur.latMin) / 2 || 0.05
            const halfLon = (cur.lonMax - cur.lonMin) / 2 || 0.08
            cb.current.onBbox?.(
                round4({latMin: lat - halfLat, latMax: lat + halfLat, lonMin: lng - halfLon, lonMax: lng + halfLon}),
            )
        } else {
            cb.current.onCircle?.({lat: round(lat), lng: round(lng), km: latest.current.circle!.km})
        }
        map.setView([lat, lng], map.getZoom())
    }

    const currentCenter = (): [number, number] => {
        const map = mapRef.current!
        if (mode === 'point') {
            const p = latest.current.point
            if (p?.lat != null && p?.lng != null) return [p.lat, p.lng]
            const c = map.getCenter()
            return [round(c.lat), round(c.lng)]
        }
        if (mode === 'bbox') {
            const b = latest.current.bbox!
            return [round((b.latMin + b.latMax) / 2), round((b.lonMin + b.lonMax) / 2)]
        }
        const c = latest.current.circle!
        return [c.lat, c.lng]
    }

    // Re-centre the map on the current point / zone (useful after panning around by hand).
    const recenter = () => {
        const map = mapRef.current
        if (!map) return
        if (mode === 'point') {
            // With extra markers, frame them all; otherwise just re-centre on the pin.
            if (extraRef.current?.length) fitPoints.current()
            else {
                const p = latest.current.point
                if (p?.lat != null && p?.lng != null) map.setView([p.lat, p.lng], 13)
            }
        } else if (mode === 'bbox') {
            const b = latest.current.bbox!
            map.fitBounds(boundsOf(b), {padding: [20, 20], maxZoom: 13})
        } else {
            const c = latest.current.circle!
            map.setView([c.lat, c.lng], zoomForKm(c.km))
        }
    }

    // Init map + the mode's overlay once Leaflet is ready.
    useEffect(() => {
        let cancelled = false
        let sizeTimer: ReturnType<typeof setTimeout> | undefined
        loadLeaflet().then((L) => {
            if (cancelled || !containerRef.current || mapRef.current) return
            const map = L.map(
                containerRef.current,
                {zoomControl: false, attributionControl: false}
            )
            map.setView([20, 0], 2)
            const bm = basemapById(basemapRef.current)
            tileRef.current = L.tileLayer(bm.url, {subdomains: bm.subdomains ?? 'abc', maxZoom: bm.maxZoom}).addTo(map)
            mapRef.current = map

            const dot = (color: string) =>
                L.divIcon({className: '', html: HANDLE_HTML(color), iconSize: [16, 16], iconAnchor: [8, 8]})

            if (mode === 'point') {
                const p = latest.current.point
                if (p?.lat != null && p?.lng != null) {
                    pin.current = L.marker([p.lat, p.lng], {draggable: interactive, icon: dot('#10b981')}).addTo(map)
                    if (interactive)
                        pin.current.on('dragend', () => {
                            const ll = pin.current!.getLatLng()
                            cb.current.onPoint?.(round(ll.lat), round(ll.lng))
                        })
                    map.setView([p.lat, p.lng], 13)
                }
                if (interactive) map.on('click', (e) => cb.current.onPoint?.(round(e.latlng.lat), round(e.latlng.lng)))
            } else if (mode === 'bbox') {
                let b = latest.current.bbox!
                if (degenerate(b)) {
                    b = boxAround(map.getCenter())
                    cb.current.onBbox?.(b)
                }
                rect.current = L.rectangle(boundsOf(b), {color: '#10b981', weight: 2}).addTo(map)
                if (!interactive) {
                    handles.current = []
                    map.fitBounds(boundsOf(b), {padding: [20, 20], maxZoom: 13})
                } else {
                    const sw = L.marker([b.latMin, b.lonMin], {draggable: true, icon: dot('#10b981')}).addTo(map)
                    const ne = L.marker([b.latMax, b.lonMax], {draggable: true, icon: dot('#10b981')}).addTo(map)
                    const center = L.marker([(b.latMin + b.latMax) / 2, (b.lonMin + b.lonMax) / 2], {
                        draggable: true,
                        icon: dot('#0ea5e9'),
                    }).addTo(map)
                    handles.current = [sw, ne, center]

                    const fromCorners = () => {
                        const a = sw.getLatLng()
                        const c = ne.getLatLng()
                        return {
                            latMin: Math.min(a.lat, c.lat),
                            latMax: Math.max(a.lat, c.lat),
                            lonMin: Math.min(a.lng, c.lng),
                            lonMax: Math.max(a.lng, c.lng),
                        }
                    }
                    const syncCornerDrag = () => {
                        const nb = round4(fromCorners())
                        rect.current!.setBounds(boundsOf(nb))
                        center.setLatLng([(nb.latMin + nb.latMax) / 2, (nb.lonMin + nb.lonMax) / 2])
                        cb.current.onBbox?.(nb)
                    }
                    // Move the box (keeping its size) to a new centre.
                    const moveBoxCenter = (lat: number, lng: number) => {
                        const cur = latest.current.bbox!
                        const halfLat = (cur.latMax - cur.latMin) / 2
                        const halfLon = (cur.lonMax - cur.lonMin) / 2
                        const nb = round4({
                            latMin: lat - halfLat,
                            latMax: lat + halfLat,
                            lonMin: lng - halfLon,
                            lonMax: lng + halfLon,
                        })
                        sw.setLatLng([nb.latMin, nb.lonMin])
                        ne.setLatLng([nb.latMax, nb.lonMax])
                        center.setLatLng([(nb.latMin + nb.latMax) / 2, (nb.lonMin + nb.lonMax) / 2])
                        rect.current!.setBounds(boundsOf(nb))
                        cb.current.onBbox?.(nb)
                    }
                    sw.on('drag', syncCornerDrag)
                    ne.on('drag', syncCornerDrag)
                    center.on('drag', () => {
                        const ll = center.getLatLng()
                        moveBoxCenter(ll.lat, ll.lng)
                    })
                    // Clicking the map moves the box centre there.
                    map.on('click', (e) => moveBoxCenter(e.latlng.lat, e.latlng.lng))
                    map.setView([(b.latMin + b.latMax) / 2, (b.lonMin + b.lonMax) / 2], 9)
                }
            } else if (mode === 'circle') {
                let c = latest.current.circle!
                if (c.lat === 0 && c.lng === 0) {
                    const ctr = map.getCenter()
                    c = {lat: round(ctr.lat), lng: round(ctr.lng), km: c.km || 10}
                    cb.current.onCircle?.(c)
                }
                circ.current = L.circle([c.lat, c.lng], {radius: c.km * 1000, color: '#10b981', weight: 2}).addTo(map)
                const center = L.marker([c.lat, c.lng], {draggable: true, icon: dot('#0ea5e9')}).addTo(map)
                const edge = L.marker(edgePoint(c), {draggable: true, icon: dot('#10b981')}).addTo(map)
                handles.current = [center, edge]

                // Move the circle centre, keeping its radius.
                const moveCircleCenter = (lat: number, lng: number) => {
                    const cur = latest.current.circle!
                    const nc = {lat: round(lat), lng: round(lng), km: cur.km}
                    circ.current!.setLatLng([nc.lat, nc.lng])
                    center.setLatLng([nc.lat, nc.lng])
                    edge.setLatLng(edgePoint(nc))
                    cb.current.onCircle?.(nc)
                }
                center.on('drag', () => {
                    const ll = center.getLatLng()
                    moveCircleCenter(ll.lat, ll.lng)
                })
                // Clicking the map moves the circle centre there.
                map.on('click', (e) => moveCircleCenter(e.latlng.lat, e.latlng.lng))
                edge.on('drag', () => {
                    const cur = latest.current.circle!
                    const meters = map.distance([cur.lat, cur.lng], edge.getLatLng())
                    const nc = {lat: cur.lat, lng: cur.lng, km: Math.max(0.1, round2(meters / 1000))}
                    circ.current!.setRadius(nc.km * 1000)
                    cb.current.onCircle?.(nc)
                })
                map.setView([c.lat, c.lng], zoomForKm(c.km))
            }

            renderFavorites.current()
            renderExtra.current()
            // Defer sizing to the next frame: the map may still be laid out (sidebar transitions), and
            // fitting bounds before the container has a real size can throw. Guard against the map being
            // torn down (fast unmount/remount when the fix panel switches targets) in the interval —
            // otherwise `invalidateSize`/`fitBounds` run against a removed map (`_leaflet_pos` undefined).
            sizeTimer = setTimeout(() => {
                if (cancelled || !mapRef.current) return
                map.invalidateSize()
                // With extra markers present, frame all points rather than just centring on the pin.
                if (extraRef.current?.length) fitPoints.current()
            }, 60)
        })
        return () => {
            cancelled = true
            if (sizeTimer) clearTimeout(sizeTimer)
            mapRef.current?.remove()
            mapRef.current = null
            pin.current = null
            rect.current = null
            circ.current = null
            handles.current = []
            favMarkers.current = []
            extraLayers.current = []
            tileRef.current = null
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [mode])

    // Swap tiles when the user picks a different basemap.
    useEffect(() => {
        const map = mapRef.current
        const L = window.L
        if (!map || !L) return
        const bm = basemapById(basemap)
        tileRef.current?.remove()
        tileRef.current = L.tileLayer(bm.url, {subdomains: bm.subdomains ?? 'abc', maxZoom: bm.maxZoom}).addTo(map)
    }, [basemap])

    // Reflect external value changes (manual inputs) onto the existing layers.
    useEffect(() => {
        const map = mapRef.current
        const L = window.L
        if (!map || !L) return
        if (mode === 'point' && point) {
            if (point.lat != null && point.lng != null) {
                if (pin.current) pin.current.setLatLng([point.lat, point.lng])
                else {
                    pin.current = L.marker([point.lat, point.lng], {
                        draggable: true,
                        icon: L.divIcon({className: '', html: HANDLE_HTML('#10b981'), iconSize: [16, 16], iconAnchor: [8, 8]}),
                    }).addTo(map)
                    pin.current.on('dragend', () => {
                        const ll = pin.current!.getLatLng()
                        cb.current.onPoint?.(round(ll.lat), round(ll.lng))
                    })
                }
            }
        } else if (mode === 'bbox' && bbox && rect.current && handles.current.length === 3) {
            rect.current.setBounds(boundsOf(bbox))
            handles.current[0].setLatLng([bbox.latMin, bbox.lonMin])
            handles.current[1].setLatLng([bbox.latMax, bbox.lonMax])
            handles.current[2].setLatLng([(bbox.latMin + bbox.latMax) / 2, (bbox.lonMin + bbox.lonMax) / 2])
        } else if (mode === 'circle' && circle && circ.current && handles.current.length === 2) {
            circ.current.setLatLng([circle.lat, circle.lng])
            circ.current.setRadius(circle.km * 1000)
            handles.current[0].setLatLng([circle.lat, circle.lng])
            handles.current[1].setLatLng(edgePoint(circle))
        }
    }, [mode, point, bbox, circle])

    // Re-render favourite pins whenever the list changes (no-op until the map is ready).
    useEffect(() => {
        renderFavorites.current()
    }, [favorites])

    // Re-render the static extra markers when they change, and re-frame all points (no-op until ready).
    useEffect(() => {
        renderExtra.current()
        if (extraMarkers?.length) fitPoints.current()
    }, [extraMarkers])

    const handleMyLocation = () => {
        if (!navigator.geolocation) return
        navigator.geolocation.getCurrentPosition((pos) =>
            applyRef.current(round(pos.coords.latitude), round(pos.coords.longitude)),
        )
    }

    const handleSaveHere = () => {
        const [lat, lng] = currentCenter()
        const id = addFav(lat, lng)
        setDraftName(coordName(lat, lng))
        setEditingFav(id)
    }

    const stop = (e: React.SyntheticEvent) => e.stopPropagation()

    return (
        // `isolate` contains Leaflet's internal z-indexes (panes/controls reach ~1000) within this
        // wrapper's own stacking context, so the map can't paint above the selection bar or dialogs.
        <div className="isolate space-y-1.5">
            <div className="relative mb-0">
                <div ref={containerRef} className={className ?? 'h-72 w-full'}/>

                {/* Basemap style switcher — top-left. */}
                <div className="absolute left-2 bottom-2 z-[1000]" onPointerDown={stop} onMouseDown={stop} onDoubleClick={stop}>
                    <select
                        value={basemap}
                        onChange={(e) => setBasemap(e.target.value)}
                        title="Map style"
                        className="h-7 rounded-md border bg-background/90 px-1.5 text-xs text-foreground shadow-sm backdrop-blur focus:outline-none"
                    >
                        {BASEMAPS.map((b) => (
                            <option key={b.id} value={b.id}>
                                {b.label}
                            </option>
                        ))}
                    </select>
                </div>

                {/* Action controls — top-right; kept above tiles and stop events reaching the map. */}
                <div
                    className="absolute right-2 top-2 z-[1000] flex flex-col gap-1"
                    onPointerDown={stop}
                    onMouseDown={stop}
                    onDoubleClick={stop}
                >
                    <button
                        type="button"
                        onClick={() => mapRef.current?.zoomIn()}
                        title="Zoom in"
                        className="flex h-7 w-7 items-center justify-center rounded-md border bg-background/90 text-foreground shadow-sm backdrop-blur hover:bg-accent"
                    >
                        <Plus className="h-4 w-4"/>
                    </button>
                    <button
                        type="button"
                        onClick={() => mapRef.current?.zoomOut()}
                        title="Zoom out"
                        className="flex h-7 w-7 items-center justify-center rounded-md border bg-background/90 text-foreground shadow-sm backdrop-blur hover:bg-accent"
                    >
                        <Minus className="h-4 w-4"/>
                    </button>
                    <button
                        type="button"
                        onClick={recenter}
                        title="Re-center on the selection"
                        className="flex h-7 w-7 items-center justify-center rounded-md border bg-background/90 text-foreground shadow-sm backdrop-blur hover:bg-accent"
                    >
                        <Crosshair className="h-4 w-4"/>
                    </button>
                    <button
                        type="button"
                        onClick={handleMyLocation}
                        title="Center on my location"
                        className="flex h-7 w-7 items-center justify-center rounded-md border bg-background/90 text-foreground shadow-sm backdrop-blur hover:bg-accent"
                    >
                        <LocateFixed className="h-4 w-4"/>
                    </button>
                    {interactive && (
                        <button
                            type="button"
                            onClick={handleSaveHere}
                            title="Save this location to favourites"
                            className="flex h-7 w-7 items-center justify-center rounded-md border bg-background/90 text-amber-500 shadow-sm backdrop-blur hover:bg-accent"
                        >
                            <Star className="h-4 w-4"/>
                        </button>
                    )}
                    {expandable && (
                        <button
                            type="button"
                            onClick={() => setExpanded(true)}
                            title="Enlarge map"
                            className="flex h-7 w-7 items-center justify-center rounded-md border bg-background/90 text-foreground shadow-sm backdrop-blur hover:bg-accent"
                        >
                            <Maximize2 className="h-4 w-4"/>
                        </button>
                    )}
                </div>
            </div>

            {interactive && (
                <FavoritesStrip
                    favorites={favorites}
                    editingId={editingFav}
                    draftName={draftName}
                    onDraftChange={setDraftName}
                    onApply={(f) => applyRef.current(f.lat, f.lng)}
                    onStartEdit={(f) => {
                        setDraftName(f.name)
                        setEditingFav(f.id)
                    }}
                    onCommitEdit={(id) => {
                        renameFav(id, draftName)
                        setEditingFav(null)
                    }}
                    onCancelEdit={() => setEditingFav(null)}
                    onRemove={removeFav}
                />
            )}
            <p className="text-[10px] mx-2 text-muted-foreground">{basemapById(basemap).attribution}</p>

            {/* Fullscreen: a second, larger map bound to the same value (a fresh Leaflet instance). */}
            {expandable && (
                <Dialog open={expanded} onOpenChange={setExpanded}>
                    <DialogContent className="max-w-5xl">
                        <MapView
                            mode={mode}
                            point={point}
                            onPoint={onPoint}
                            bbox={bbox}
                            onBbox={onBbox}
                            circle={circle}
                            onCircle={onCircle}
                            className="h-[70vh] w-full"
                            expandable={false}
                            extraMarkers={extraMarkers}
                        />
                    </DialogContent>
                </Dialog>
            )}
        </div>
    )
}

// ── Favourites strip ─────────────────────────────────────────────────────────────

function FavoritesStrip({
                            favorites,
                            editingId,
                            draftName,
                            onDraftChange,
                            onApply,
                            onStartEdit,
                            onCommitEdit,
                            onCancelEdit,
                            onRemove,
                        }: {
    favorites: FavoriteLocation[]
    editingId: string | null
    draftName: string
    onDraftChange: (v: string) => void
    onApply: (f: FavoriteLocation) => void
    onStartEdit: (f: FavoriteLocation) => void
    onCommitEdit: (id: string) => void
    onCancelEdit: () => void
    onRemove: (id: string) => void
}) {
    if (favorites.length === 0) {
        return <p className="text-[11px] text-muted-foreground">Use ★ to save the current location.</p>
    }
    return (
        <div className="flex flex-wrap gap-1">
            {favorites.map((f) =>
                editingId === f.id ? (
                    <span key={f.id} className="flex items-center gap-1 rounded-full border bg-background px-1.5 py-0.5">
                        <input
                            autoFocus
                            value={draftName}
                            onChange={(e) => onDraftChange(e.target.value)}
                            onKeyDown={(e) => {
                                if (e.key === 'Enter') onCommitEdit(f.id)
                                if (e.key === 'Escape') onCancelEdit()
                            }}
                            className="w-28 bg-transparent text-xs outline-none"
                        />
                        <button onClick={() => onCommitEdit(f.id)} className="text-emerald-500" aria-label="Save name">
                            <Check className="h-3 w-3"/>
                        </button>
                        <button onClick={onCancelEdit} className="text-muted-foreground" aria-label="Cancel">
                            <X className="h-3 w-3"/>
                        </button>
                    </span>
                ) : (
                    <span
                        key={f.id}
                        className="group flex items-center gap-1 rounded-full border bg-background px-2 py-0.5 text-xs"
                    >
                        <button onClick={() => onApply(f)} className="flex items-center gap-1 hover:text-primary" title="Center here">
                            <Star className="h-3 w-3 text-amber-500"/>
                            {f.name}
                        </button>
                        <button
                            onClick={() => onStartEdit(f)}
                            className="text-muted-foreground/60 hover:text-foreground"
                            aria-label="Rename"
                        >
                            <Pencil className="h-3 w-3"/>
                        </button>
                        <button
                            onClick={() => onRemove(f.id)}
                            className="text-muted-foreground/60 hover:text-destructive"
                            aria-label="Delete favourite"
                        >
                            <Trash2 className="h-3 w-3"/>
                        </button>
                    </span>
                ),
            )}
        </div>
    )
}

// ── geometry helpers ─────────────────────────────────────────────────────────────

const round = (n: number) => Math.round(n * 1e6) / 1e6
const round4 = (b: Bbox): Bbox => ({
    latMin: r4(b.latMin),
    latMax: r4(b.latMax),
    lonMin: r4(b.lonMin),
    lonMax: r4(b.lonMax),
})
const r4 = (n: number) => Math.round(n * 1e4) / 1e4
const round2 = (n: number) => Math.round(n * 100) / 100

const boundsOf = (b: Bbox): [[number, number], [number, number]] => [
    [b.latMin, b.lonMin],
    [b.latMax, b.lonMax],
]

const degenerate = (b: Bbox) => b.latMin === b.latMax && b.lonMin === b.lonMax

function boxAround(c: LatLng): Bbox {
    return {latMin: r4(c.lat - 0.05), latMax: r4(c.lat + 0.05), lonMin: r4(c.lng - 0.08), lonMax: r4(c.lng + 0.08)}
}

/** A point on the circle's eastern edge, used as the resize handle position. */
function edgePoint(c: CircleZone): [number, number] {
    const dLon = c.km / (111.32 * Math.cos((c.lat * Math.PI) / 180) || 1)
    return [c.lat, c.lng + dLon]
}

function zoomForKm(km: number): number {
    if (km <= 2) return 12
    if (km <= 10) return 10
    if (km <= 50) return 8
    if (km <= 200) return 6
    return 4
}
