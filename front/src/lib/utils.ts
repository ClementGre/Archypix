import {type ClassValue, clsx} from "clsx"
import {twMerge} from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
    return twMerge(clsx(inputs))
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
