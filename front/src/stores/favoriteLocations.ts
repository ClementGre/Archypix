import {create} from 'zustand'

const LS_KEY = 'archypix_fav_locations'

export interface FavoriteLocation {
    id: string
    name: string
    lat: number
    lng: number
}

function load(): FavoriteLocation[] {
    try {
        const raw = localStorage.getItem(LS_KEY)
        if (raw) return JSON.parse(raw) as FavoriteLocation[]
    } catch {
        // ignore malformed storage
    }
    return []
}

function persist(favorites: FavoriteLocation[]) {
    try {
        localStorage.setItem(LS_KEY, JSON.stringify(favorites))
    } catch {
        // ignore quota / privacy-mode failures
    }
}

/** Default display name for a freshly-saved location: its coordinates. */
export function coordName(lat: number, lng: number): string {
    return `${lat.toFixed(4)}, ${lng.toFixed(4)}`
}

interface FavoriteLocationsState {
    favorites: FavoriteLocation[]
    /** Save a point and return its id (caller may then rename it inline). */
    add: (lat: number, lng: number, name?: string) => string
    rename: (id: string, name: string) => void
    remove: (id: string) => void
}

export const useFavoriteLocations = create<FavoriteLocationsState>((set, get) => ({
    favorites: load(),
    add: (lat, lng, name) => {
        const id = crypto.randomUUID()
        const fav: FavoriteLocation = {id, name: name?.trim() || coordName(lat, lng), lat, lng}
        const favorites = [...get().favorites, fav]
        persist(favorites)
        set({favorites})
        return id
    },
    rename: (id, name) => {
        const favorites = get().favorites.map((f) => (f.id === id ? {...f, name: name.trim() || f.name} : f))
        persist(favorites)
        set({favorites})
    },
    remove: (id) => {
        const favorites = get().favorites.filter((f) => f.id !== id)
        persist(favorites)
        set({favorites})
    },
}))
