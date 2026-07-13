import {useEffect, useMemo, useRef, useState} from 'react'
import {AlertTriangle, Check, Loader2, UserRound} from 'lucide-react'
import {useIncomingShares, useOutgoingShares} from '@/hooks/useShares'
import {checkIdentityExists, type IdentityCheck} from '@/api/resolve'
import {useAuthStore} from '@/stores/auth'
import {useDebouncedValue} from '@/hooks/useDebouncedValue'
import {formatIdentity, type Identity, parseIdentity} from '@/lib/identity'
import {cn} from '@/lib/utils'

export interface ContactInputProps {
    /** Current value: an `@user:domain` identity, or (when `allowCustomValues`) a plain-text credit. */
    value: string
    onChange: (value: string) => void
    /**
     * When true, arbitrary plain text is allowed **as long as it doesn't start with `@` or `#`**; a
     * leading `@` switches to identity mode (contact autocomplete + resolver check). When false, the
     * field is identity-only with a hardcoded `@` prefix.
     */
    allowCustomValues: boolean
    /** Include the logged-in user's own identity in the autocomplete (on for the creator, off for shares). */
    includeSelf?: boolean
    /** Instance used when an identity is typed without an explicit `:domain` (identity-only fields). */
    defaultInstance?: string
    /** Reports structural + verification validity so the parent can gate submit. Empty is valid. */
    onValidityChange?: (valid: boolean) => void
    onEnter?: () => void
    onEscape?: () => void
    placeholder?: string
    disabled?: boolean
    autoFocus?: boolean
    className?: string
}

/** The identity a value resolves to (respecting a default instance), or null if not a full identity. */
function effectiveIdentity(value: string, defaultInstance?: string): Identity | null {
    if (!value.trimStart().startsWith('@')) return null
    return parseIdentity(value, defaultInstance)
}

/**
 * The domain the user has actually **typed** as `…:domain` with at least one dot (a syntactically
 * plausible domain), else null. Gates verification + the "unreachable" hint so a half-typed domain
 * (`@alice:e`) or a defaulted instance (`@alice`, no `:`) never flashes an error.
 */
function typedDomainOf(value: string): string | null {
    const body = value.trim().replace(/^@/, '')
    const idx = body.indexOf(':')
    if (idx === -1) return null
    const domain = body.slice(idx + 1)
    return domain.includes('.') ? domain : null
}

/**
 * A user/contact autocomplete input (feature 26). Suggests `@user:domain` from the user's incoming +
 * outgoing share contacts and verifies a typed identity against the resolver (best-effort, advisory).
 * Reused by the creator field (`allowCustomValues`) and the create-share recipient rows (identity-only).
 */
export function ContactInput({
                                 value,
                                 onChange,
                                 allowCustomValues,
                                 includeSelf,
                                 defaultInstance,
                                 onValidityChange,
                                 onEnter,
                                 onEscape,
                                 placeholder,
                                 disabled,
                                 autoFocus,
                                 className,
                             }: ContactInputProps) {
    const {data: incoming} = useIncomingShares()
    const {data: outgoing} = useOutgoingShares()
    const currentUser = useAuthStore((s) => s.user)
    const currentInstance = useAuthStore((s) => s.instance)
    const [open, setOpen] = useState(false)
    const [highlight, setHighlight] = useState(0)
    const inputRef = useRef<HTMLInputElement>(null)

    // Distinct `@user:domain` contacts from both share directions. The user's own identity is always
    // stripped out of the share-partner set (it can appear via a self share-back), then re-added at
    // the front only when `includeSelf` is on (creator field; off for share recipients).
    const contacts = useMemo(() => {
        const self =
            currentUser && currentInstance
                ? formatIdentity({username: currentUser.username, instance: currentInstance})
                : null
        const selfLower = self?.toLowerCase()
        const others = new Set<string>()
        for (const s of incoming ?? []) others.add(formatIdentity({username: s.sender_username, instance: s.sender_instance}))
        for (const s of outgoing ?? []) others.add(formatIdentity({username: s.recipient_username, instance: s.recipient_instance}))
        const sorted = [...others].filter((c) => c.toLowerCase() !== selfLower).sort()
        return includeSelf && self ? [self, ...sorted] : sorted
    }, [incoming, outgoing, includeSelf, currentUser, currentInstance])

    // Identity-only fields store the value with a fixed leading `@`; the editable text drops it.
    const identityOnly = !allowCustomValues
    const text = identityOnly ? value.replace(/^@/, '') : value

    const startsHash = value.trimStart().startsWith('#')
    const isIdentityMode = identityOnly || value.trimStart().startsWith('@')
    const identity = isIdentityMode ? effectiveIdentity(identityOnly ? `@${text}` : value, defaultInstance) : null

    // Suggestions: filter contacts by the identity text being typed.
    const suggestions = useMemo(() => {
        if (!isIdentityMode) return []
        const needle = (identityOnly ? text : value.replace(/^@/, '')).toLowerCase()
        return contacts.filter((c) => c.toLowerCase().includes(needle)).slice(0, 8)
    }, [contacts, isIdentityMode, identityOnly, text, value])

    // Verify only once a real domain has been typed (`…:domain.tld`) — not for a defaulted instance
    // or a half-typed domain. Any failure is `unreachable` (advisory, never blocks submit).
    const typedDomain = useMemo(() => (isIdentityMode ? typedDomainOf(value) : null), [isIdentityMode, value])
    const debouncedIdentity = useDebouncedValue(identity && typedDomain ? formatIdentity(identity) : '', 500)
    const [check, setCheck] = useState<IdentityCheck | 'checking' | null>(null)
    useEffect(() => {
        const id = debouncedIdentity ? parseIdentity(debouncedIdentity) : null
        if (!id) {
            setCheck(null)
            return
        }
        let cancelled = false
        setCheck('checking')
        void checkIdentityExists(id.username, id.instance).then((r) => {
            if (!cancelled) setCheck(r)
        })
        return () => {
            cancelled = true
        }
    }, [debouncedIdentity])

    // Validity is purely structural (a flaky resolver check never blocks; the real op is authoritative).
    const valid = value.trim() === '' ? true : startsHash ? false : isIdentityMode ? identity !== null : true
    useEffect(() => {
        onValidityChange?.(valid)
    }, [valid, onValidityChange])

    const commit = (raw: string) => {
        if (identityOnly) {
            // Strip stray `@` (the prefix is fixed) and whitespace.
            onChange(`@${raw.replace(/@/g, '').replace(/\s+/g, '')}`)
        } else {
            onChange(raw)
        }
        setHighlight(0)
        setOpen(true)
    }

    const pick = (contact: string) => {
        onChange(contact) // contacts are already full `@user:domain`
        setOpen(false)
        inputRef.current?.focus()
    }

    const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
        if (open && suggestions.length > 0) {
            if (e.key === 'ArrowDown') {
                e.preventDefault()
                setHighlight((h) => (h + 1) % suggestions.length)
                return
            }
            if (e.key === 'ArrowUp') {
                e.preventDefault()
                setHighlight((h) => (h - 1 + suggestions.length) % suggestions.length)
                return
            }
        }
        if (e.key === 'Enter') {
            e.preventDefault()
            if (open && suggestions.length > 0 && suggestions[highlight]) pick(suggestions[highlight])
            else onEnter?.()
            return
        }
        if (e.key === 'Escape') {
            if (open) {
                e.preventDefault()
                setOpen(false)
            } else onEscape?.()
        }
    }

    const showSuggestions = open && isIdentityMode && suggestions.length > 0

    return (
        <div className={cn('relative min-w-0', className)}>
            <div
                className={cn(
                    'flex items-center gap-1 rounded-md border border-input bg-background px-2 text-sm',
                    'focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-2 focus-within:ring-offset-background',
                    disabled && 'opacity-50',
                )}
            >
                {identityOnly && <span className="select-none text-muted-foreground">@</span>}
                <input
                    ref={inputRef}
                    value={text}
                    disabled={disabled}
                    autoFocus={autoFocus}
                    onChange={(e) => commit(e.target.value)}
                    onFocus={() => setOpen(true)}
                    onBlur={() => setOpen(false)}
                    onKeyDown={onKeyDown}
                    placeholder={placeholder ?? (identityOnly ? 'username:domain' : 'Add a credit…')}
                    autoCapitalize="none"
                    autoCorrect="off"
                    spellCheck={false}
                    className="min-w-0 flex-1 bg-transparent py-1.5 outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
                />
                {check === 'checking' && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground"/>}
                {check === 'exists' && <Check className="h-3.5 w-3.5 shrink-0 text-emerald-500"/>}
            </div>

            {showSuggestions && (
                <ul className="absolute z-50 mt-1 max-h-56 w-full overflow-y-auto rounded-md border border-border bg-popover py-1 text-sm shadow-md">
                    {suggestions.map((c, i) => (
                        <li key={c}>
                            <button
                                type="button"
                                // mousedown fires before blur; keep focus and select.
                                onMouseDown={(e) => {
                                    e.preventDefault()
                                    pick(c)
                                }}
                                onMouseEnter={() => setHighlight(i)}
                                className={cn(
                                    'flex w-full items-center gap-2 px-2 py-1.5 text-left',
                                    i === highlight ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/50',
                                )}
                            >
                                <UserRound className="h-3.5 w-3.5 shrink-0 opacity-60"/>
                                <span className="min-w-0 truncate">{c}</span>
                            </button>
                        </li>
                    ))}
                </ul>
            )}

            {/* Inline validation / verification hints. */}
            {startsHash && (
                <p className="mt-1 flex items-start gap-1 text-[11px] text-destructive">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                    <span>“#” is reserved for public-share contributions.</span>
                </p>
            )}
            {!startsHash && isIdentityMode && value.trim() !== '' && !identity && (
                <p className="mt-1 text-[11px] text-muted-foreground">Type a full <code>@username:domain</code>.</p>
            )}
            {check === 'unreachable' && typedDomain && (
                <p className="mt-1 flex items-start gap-1 text-[11px] text-destructive">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                    <span><code>{typedDomain}</code> is unreachable — check the domain.</span>
                </p>
            )}
            {check === 'missing' && typedDomain && (
                <p className="mt-1 flex items-start gap-1 text-[11px] text-destructive">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                    <span>No account {identity ? formatIdentity(identity) : ''} on <code>{typedDomain}</code>.</span>
                </p>
            )}
        </div>
    )
}
