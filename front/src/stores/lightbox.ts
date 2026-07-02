import {create} from 'zustand'

const LS_KEY = 'archypix_lightbox'

/**
 * Lightbox chrome preferences. The top-bar and carousel visibility are kept **separately** for
 * fullscreen vs. non-fullscreen (persisted): chrome is shown by default in a normal window and
 * hidden by default in fullscreen. Original-quality is session-only (defaults off on every load,
 * never persisted, shared across both modes — see the feature request).
 */
interface Persisted {
    topBarNormal: boolean
    topBarFullscreen: boolean
    carouselNormal: boolean
    carouselFullscreen: boolean
}

const DEFAULTS: Persisted = {
    topBarNormal: true,
    topBarFullscreen: false,
    carouselNormal: true,
    carouselFullscreen: false,
}

function load(): Persisted {
    try {
        const raw = localStorage.getItem(LS_KEY)
        if (raw) return {...DEFAULTS, ...(JSON.parse(raw) as Partial<Persisted>)}
    } catch {
        // ignore
    }
    return DEFAULTS
}

function save(s: Persisted) {
    try {
        localStorage.setItem(
            LS_KEY,
            JSON.stringify({
                topBarNormal: s.topBarNormal,
                topBarFullscreen: s.topBarFullscreen,
                carouselNormal: s.carouselNormal,
                carouselFullscreen: s.carouselFullscreen,
            }),
        )
    } catch {
        // ignore
    }
}

interface LightboxState extends Persisted {
    /** Whether the browser is in fullscreen for the lightbox (mirrors `document.fullscreenElement`). */
    fullscreen: boolean
    /** Request the original blob at presign time instead of the `large` thumbnail (session-only). */
    originalQuality: boolean
    setFullscreen: (v: boolean) => void
    toggleTopBar: () => void
    toggleCarousel: () => void
    toggleOriginalQuality: () => void
}

export const useLightboxStore = create<LightboxState>((set, get) => ({
    ...load(),
    fullscreen: false,
    originalQuality: false,
    setFullscreen: (v) => set({fullscreen: v}),
    toggleTopBar: () => {
        const s = get()
        const next = s.fullscreen ? {topBarFullscreen: !s.topBarFullscreen} : {topBarNormal: !s.topBarNormal}
        set(next)
        save({...s, ...next})
    },
    toggleCarousel: () => {
        const s = get()
        const next = s.fullscreen ? {carouselFullscreen: !s.carouselFullscreen} : {carouselNormal: !s.carouselNormal}
        set(next)
        save({...s, ...next})
    },
    toggleOriginalQuality: () => set((s) => ({originalQuality: !s.originalQuality})),
}))

/** Derived: is the top bar shown for the current (fullscreen or not) mode? */
export const topBarVisible = (s: LightboxState) => (s.fullscreen ? s.topBarFullscreen : s.topBarNormal)
/** Derived: is the carousel shown for the current mode? */
export const carouselVisible = (s: LightboxState) => (s.fullscreen ? s.carouselFullscreen : s.carouselNormal)
