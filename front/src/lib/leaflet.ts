// Shared vanilla-Leaflet loader and a minimal typed surface, used by the factored `MapView`
// component (point / bbox / circle pickers). We deliberately avoid the react-leaflet wrapper
// (it pulled a duplicate React copy and crashed with "Invalid hook call"); plain Leaflet has no
// React dependency and is driven imperatively.

export interface LatLng {
    lat: number
    lng: number

    distanceTo(other: LatLng): number
}

export interface LeafletMouseEvent {
    latlng: LatLng
}

export interface LLayer {
    addTo(map: LMap): this

    remove(): void
}

export interface LMap {
    setView(center: [number, number], zoom?: number): LMap

    fitBounds(bounds: [[number, number], [number, number]], options?: { padding?: [number, number]; maxZoom?: number }): LMap

    zoomIn(delta?: number): LMap

    zoomOut(delta?: number): LMap

    on(event: string, cb: (e: LeafletMouseEvent) => void): LMap

    remove(): void

    invalidateSize(): void

    distance(a: [number, number] | LatLng, b: [number, number] | LatLng): number

    getCenter(): LatLng

    getZoom(): number
}

export interface LMarker extends LLayer {
    setLatLng(center: [number, number]): LMarker

    getLatLng(): LatLng

    on(event: string, cb: () => void): LMarker

    bindTooltip(content: string, opts?: Record<string, unknown>): LMarker
}

export interface LRectangle extends LLayer {
    setBounds(bounds: [[number, number], [number, number]]): LRectangle
}

export interface LCircle extends LLayer {
    setLatLng(center: [number, number]): LCircle

    setRadius(meters: number): LCircle

    getRadius(): number
}

export interface LeafletStatic {
    map(el: HTMLElement, opts?: Record<string, unknown>): LMap

    tileLayer(url: string, opts?: Record<string, unknown>): LLayer

    marker(center: [number, number], opts?: Record<string, unknown>): LMarker

    rectangle(bounds: [[number, number], [number, number]], opts?: Record<string, unknown>): LRectangle

    circle(center: [number, number], opts?: Record<string, unknown>): LCircle

    latLng(lat: number, lng: number): LatLng

    divIcon(opts: Record<string, unknown>): unknown
}

declare global {
    interface Window {
        L?: LeafletStatic
    }
}

let leafletPromise: Promise<LeafletStatic> | null = null

/** Load Leaflet (CSS + JS) once from CDN; resolves the global `L`. */
export function loadLeaflet(): Promise<LeafletStatic> {
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

export const OSM_TILE_URL = 'https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png'

/** A selectable basemap (all free, no API key; attribution required). */
export interface Basemap {
    id: string
    label: string
    url: string
    subdomains?: string
    attribution: string
    maxZoom: number
}

export const BASEMAPS: Basemap[] = [
    {
        // CARTO Voyager — detailed streets/labels, the default. Much richer than Positron/Dark Matter.
        id: 'voyager',
        label: 'Streets',
        url: 'https://{s}.basemaps.cartocdn.com/rastertiles/voyager/{z}/{x}/{y}.png',
        subdomains: 'abcd',
        attribution: '© OpenStreetMap, © CARTO',
        maxZoom: 20,
    },
    {
        // Esri World Imagery — satellite, best for locating real places.
        id: 'satellite',
        label: 'Satellite',
        url: 'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}',
        attribution: 'Imagery © Esri',
        maxZoom: 19,
    },
    {
        // OpenStreetMap standard — maximum street detail.
        id: 'osm',
        label: 'OSM',
        url: OSM_TILE_URL,
        subdomains: 'abc',
        attribution: '© OpenStreetMap contributors',
        maxZoom: 19,
    },
    {
        id: 'light',
        label: 'Light',
        url: 'https://{s}.basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png',
        subdomains: 'abcd',
        attribution: '© OpenStreetMap, © CARTO',
        maxZoom: 20
    },
    {
        id: 'dark',
        label: 'Dark',
        url: 'https://{s}.basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png',
        subdomains: 'abcd',
        attribution: '© OpenStreetMap, © CARTO',
        maxZoom: 20
    },
]

export function basemapById(id: string): Basemap {
    return BASEMAPS.find((b) => b.id === id) ?? BASEMAPS[0]
}
