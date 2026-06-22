import {create} from 'zustand'

const LS_KEY = 'archypix_basemap'

function read(): string {
    try {
        return localStorage.getItem(LS_KEY) || 'voyager'
    } catch {
        return 'voyager'
    }
}

interface MapStyleState {
    /** Selected basemap id (see `BASEMAPS` in `lib/leaflet.ts`). Shared by every map, persisted. */
    basemap: string
    setBasemap: (id: string) => void
}

export const useMapStyle = create<MapStyleState>((set) => ({
    basemap: read(),
    setBasemap: (id) => {
        try {
            localStorage.setItem(LS_KEY, id)
        } catch {
            // ignore quota / privacy-mode failures
        }
        set({basemap: id})
    },
}))
