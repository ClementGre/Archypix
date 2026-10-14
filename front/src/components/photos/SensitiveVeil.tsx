import {type ReactNode, useEffect, useState} from 'react'
import {Eye, EyeOff, Lock, X} from 'lucide-react'
import {cn} from '@/lib/utils'
import {SHOW_ALL_PRESETS, useSensitiveStore} from '@/stores/sensitive'
import {useVeil, type Veil} from '@/hooks/useSensitive'
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuLabel,
    DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {Blurhash} from './Blurhash'
import {orientedCoverStyle} from './OrientedImage'

// Feature 38: the curtain over sensitive pictures. The image itself is never mounted while veiled.

/**
 * A veiled picture filling its `relative` parent: the blurhash, or the redacted tile (Redacted mode,
 * or no blurhash yet). `revealId` adds the reveal controls on top.
 */
export function VeilFill({veil, blurhash, orientation, width, height, revealId, className}: {
    veil: Exclude<Veil, 'visible'>
    blurhash?: string | null
    orientation?: number | null
    width?: number | null
    height?: number | null
    revealId?: string
    className?: string
}) {
    const cover = orientedCoverStyle(orientation, width, height)
    return (
        <div className={cn('absolute inset-0 overflow-hidden', className)}>
            {veil === 'blurred' && blurhash ? (
                <Blurhash hash={blurhash} className={cover.className} style={cover.style}/>
            ) : (
                <div className="absolute inset-0 flex items-center justify-center bg-redacted">
                    <EyeOff className="h-1/4 max-h-10 w-1/4 max-w-10 text-white/35"/>
                </div>
            )}
            {revealId && <RevealControls id={revealId}/>}
        </div>
    )
}

/** Centred Reveal / Show-all controls over a veiled single picture (lightbox, sidebar preview). */
function RevealControls({id}: { id: string }) {
    const setRevealed = useSensitiveStore((s) => s.setRevealed)
    return (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2" onClick={(e) => e.stopPropagation()}>
            <button
                onClick={() => setRevealed(id, true)}
                className="flex items-center gap-1.5 rounded-full bg-black/60 px-3 py-1.5 text-sm text-white hover:bg-black/75"
            >
                <Eye className="h-4 w-4"/> Reveal
            </button>
            <ShowAllMenu>
                <button className="rounded-full px-2 py-0.5 text-xs text-white/80 hover:bg-black/40 hover:text-white">
                    Show all sensitive…
                </button>
            </ShowAllMenu>
        </div>
    )
}

/** Start the tab-scoped show-all override (§4) for one of the preset durations. */
export function ShowAllMenu({children}: { children: ReactNode }) {
    const startShowAll = useSensitiveStore((s) => s.startShowAll)
    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
            <DropdownMenuContent onClick={(e) => e.stopPropagation()}>
                <DropdownMenuLabel>Show sensitive pictures in this tab</DropdownMenuLabel>
                {SHOW_ALL_PRESETS.map((m) => (
                    <DropdownMenuItem key={m} onSelect={() => startShowAll(m)}>
                        For {m} minutes
                    </DropdownMenuItem>
                ))}
            </DropdownMenuContent>
        </DropdownMenu>
    )
}

/**
 * Reveal / Hide again for one sensitive picture, in a toolbar. Renders nothing for a picture that is
 * not sensitive, or that is visible only through the show-all override or Visible mode.
 */
export function RevealToggle({id, sensitive, className}: {
    id: string
    sensitive: boolean | null | undefined
    className?: string
}) {
    const revealed = useSensitiveStore((s) => s.revealed.has(id))
    const setRevealed = useSensitiveStore((s) => s.setRevealed)
    const veil = useVeil(id, sensitive)
    if (!sensitive || (!revealed && veil === 'visible')) return null
    const label = revealed ? 'Hide again' : 'Reveal'
    return (
        <button onClick={() => setRevealed(id, !revealed)} title={label} aria-label={label} className={className}>
            {revealed ? <EyeOff className="h-4 w-4"/> : <Eye className="h-4 w-4"/>}
        </button>
    )
}

/** The always-on sensitive marker as a small pill (§3). */
export function SensitiveBadge({className}: { className?: string }) {
    return (
        <span className={cn('inline-flex items-center gap-0.5 rounded bg-red-600/90 px-1 text-[10px] font-medium text-white', className)}>
            <Lock className="h-2.5 w-2.5"/> Sensitive
        </span>
    )
}

/** Countdown + Stop for the active show-all override; renders nothing when it is off (§4). */
export function ShowAllChip() {
    const until = useSensitiveStore((s) => s.showAllUntil)
    const stop = useSensitiveStore((s) => s.stopShowAll)
    const [now, setNow] = useState(() => Date.now())
    useEffect(() => {
        if (!until) return
        const t = window.setInterval(() => setNow(Date.now()), 1000)
        return () => window.clearInterval(t)
    }, [until])
    if (!until) return null
    const left = Math.max(0, Math.round((until - now) / 1000))
    const mmss = `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`
    return (
        <span
            title="Sensitive pictures are shown in this tab"
            className="inline-flex h-7 shrink-0 items-center gap-1 rounded-full bg-red-600/90 pl-2 pr-1 text-xs font-medium tabular-nums text-white"
        >
            <Eye className="h-3.5 w-3.5"/>
            {mmss}
            <button onClick={stop} title="Hide sensitive pictures now" aria-label="Hide sensitive pictures now"
                    className="flex h-5 w-5 items-center justify-center rounded-full hover:bg-white/20">
                <X className="h-3.5 w-3.5"/>
            </button>
        </span>
    )
}
