// Archypix identity helpers — the `@username:domain` credit/recipient format (features 25/26).

export interface Identity {
    username: string
    instance: string
}

/** Render an identity as `@username:domain`. */
export function formatIdentity({username, instance}: Identity): string {
    return `@${username}:${instance}`
}

/**
 * Parse a `@username:domain` value into its parts. Accepts a leading `@` (optional). When the value
 * carries no `:instance` and a `defaultInstance` is given, that instance is used (lets a recipient be
 * typed as just `@alice`). Returns `null` when there is no username or the shape is malformed.
 */
export function parseIdentity(value: string, defaultInstance?: string): Identity | null {
    const body = value.trim().replace(/^@/, '')
    if (!body) return null
    const idx = body.indexOf(':')
    if (idx === -1) {
        return defaultInstance ? {username: body, instance: defaultInstance} : null
    }
    const username = body.slice(0, idx)
    const instance = body.slice(idx + 1)
    if (!username || !instance || instance.includes(':')) return null
    return {username, instance}
}

/** Whether `value` is an `@`-sigil identity token (as opposed to a plain-text credit). */
export function isIdentityToken(value: string): boolean {
    return value.trimStart().startsWith('@')
}
