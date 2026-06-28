import {type ClassValue, clsx} from "clsx"
import {twMerge} from "tailwind-merge"
import type {PictureVariant} from "@/lib/types"

export function cn(...inputs: ClassValue[]) {
    return twMerge(clsx(inputs))
}

// ── Thumbnails ─────────────────────────────────────────────────────────────────

/**
 * Pick a thumbnail variant (small=150, medium=500, large=1000 px tall — see worker
 * `THUMBNAIL_VARIANTS`) for an element displayed at `cssPx` CSS pixels tall. Sizing
 * is by logical pixels (no devicePixelRatio multiplier) to keep payloads light:
 * slight upscaling on hi-DPI screens is an accepted trade-off. Used by the grid
 * (zoom/row height) and sidebar; the lightbox always asks for `large`.
 */
export function variantForSize(cssPx: number): PictureVariant {
    if (cssPx <= 80) return 'small'
    if (cssPx <= 350) return 'medium'
    return 'large'
}

// ── Media (video/audio) ────────────────────────────────────────────────────────

/** A picture whose mime is a video — played inline rather than shown as an image. */
export function isVideoMime(mime: string | null | undefined): boolean {
    return !!mime && mime.toLowerCase().startsWith('video/')
}

/** A picture whose mime is audio — played in the audio-bar layout. */
export function isAudioMime(mime: string | null | undefined): boolean {
    return !!mime && mime.toLowerCase().startsWith('audio/')
}

/** Video or audio — rendered by the `MediaPlayer` instead of the image pipeline. */
export function isPlayableMedia(mime: string | null | undefined): boolean {
    return isVideoMime(mime) || isAudioMime(mime)
}

// ── Formatting ───────────────────────────────────────────────────────────────

/** Human-readable media duration (e.g. `1:05`, `1:02:03`). */
export function formatDuration(seconds: number | null | undefined): string {
    if (seconds == null || !isFinite(seconds) || seconds < 0) return '—'
    const total = Math.round(seconds)
    const h = Math.floor(total / 3600)
    const m = Math.floor((total % 3600) / 60)
    const s = total % 60
    const pad = (n: number) => String(n).padStart(2, '0')
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`
}

/** Human-readable byte size (e.g. `4.2 GB`). */
export function formatBytes(bytes: number | null | undefined): string {
    if (!bytes) return '—'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    let n = bytes
    let i = 0
    while (n >= 1024 && i < units.length - 1) {
        n /= 1024
        i++
    }
    return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${units[i]}`
}

/**
 * Format a server timestamp in the user's local timezone. Timestamps that carry
 * no timezone offset are assumed to be UTC (the backend stores them as UTC).
 */
export function formatDateTime(iso: string | null | undefined): string {
    if (!iso) return '—'
    // Append `Z` when the value has a date+time but no explicit timezone, so it is
    // parsed as UTC rather than the browser's local time.
    const hasTz = /[zZ]$|[+-]\d{2}:?\d{2}$/.test(iso)
    const normalized = !hasTz && iso.includes('T') ? `${iso}Z` : iso
    const d = new Date(normalized)
    if (Number.isNaN(d.getTime())) return '—'
    return d.toLocaleString(undefined, {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
    })
}

// ── Tag paths ────────────────────────────────────────────────────────────────
// Wire form (ltree, dot-separated): `Photos.Travel.Alps`
// Display form (slash-separated, leading slash): `/Photos/Travel/Alps`
// Special label encoding used by the backend for share identities:
//   `alice@ex.com` ⇄ `alice_AT_ex_DOT_com`

const PROTECTED_PREFIX = 'SharedToMe'

function decodeLabel(label: string): string {
    return label.replace(/_AT_/g, '@').replace(/_DOT_/g, '.')
}

function encodeLabel(label: string): string {
    return label.replace(/@/g, '_AT_').replace(/\./g, '_DOT_')
}

export const TagPath = {
    /** Wire (`Photos.Travel.Alps`) → display (`/Photos/Travel/Alps`). */
    toDisplay(wire: string): string {
        if (!wire) return ''
        return '/' + wire.split('.').map(decodeLabel).join('/')
    },

    /** Display (`/Photos/Travel/Alps`) → wire (`Photos.Travel.Alps`). */
    toWire(display: string): string {
        const trimmed = display.replace(/^\/+/, '').replace(/\/+$/, '')
        if (!trimmed) return ''
        return trimmed
            .split('/')
            .filter(Boolean)
            .map(encodeLabel)
            .join('.')
    },

    /** Decoded label segments of a wire path. */
    segments(wire: string): string[] {
        return wire ? wire.split('.').map(decodeLabel) : []
    },

    /** Deepest (leaf) decoded label of a wire path. */
    leaf(wire: string): string {
        const parts = wire.split('.')
        return decodeLabel(parts[parts.length - 1] ?? '')
    },

    /** True if the wire path is under the reserved `SharedToMe` subtree. */
    isProtected(wire: string): boolean {
        return wire === PROTECTED_PREFIX || wire.startsWith(`${PROTECTED_PREFIX}.`)
    },
}
