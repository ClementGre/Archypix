import {memo, type MouseEvent, type PointerEvent, type ReactNode, useRef, useState} from 'react'
import {AlertTriangle, Check, Clock, CloudOff, MapPin, RefreshCw, Trash2} from 'lucide-react'
import {useQueryClient} from '@tanstack/react-query'
import type {PictureListItem} from '@/lib/types'
import {getPictureUrl} from '@/api/pictures'
import {usePresignRefresh} from '@/hooks/usePresignRefresh'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {recordImage} from '@/stores/imageCache'
import {cn, isVideoMime, variantForSize} from '@/lib/utils'
import {countdown, deadlineLabel, ownedPurgeAt} from '@/lib/trash'
import {Blurhash} from './Blurhash'
import {FileTypeIcon} from './FileTypeIcon'
import {PlayBadge} from './PlayBadge'
import {displayDimensions, orientedCoverStyle, OrientedImage} from './OrientedImage'

/** Compact magnitude of a time span (ms) with an auto unit: s / min / h / d / mo / y. */
function formatDuration(ms: number): string {
    const negative = ms < 0
    if (negative) ms = -ms
    const prefix = negative ? '-' : '+'

    const s = Math.round(ms / 1000)
    if (s == 0) return '0s'
    if (s < 60) return `${prefix}${s}s`
    const m = Math.round(s / 60)
    if (m < 60) return `${prefix}${m}min`
    const h = Math.round(m / 60)
    if (h < 24) return `${prefix}${h}h`
    const d = Math.round(h / 24)
    if (d < 30) return `${prefix}${d}d`
    const mo = Math.round(d / 30)
    if (mo < 12) return `${prefix}${mo}mo`
    return `${prefix}${Math.round(d / 365)}y`
}

/**
 * A small tag sitting flush in one corner of the card (trash / owner / purge / proximity). `left` and
 * `top` pick the corner (`false` → right / bottom); only the corner facing into the card is rounded.
 * `className` is merged over the defaults, so callers can retint the background or tweak spacing.
 */
function CornerLabel({top, left, title, className, children}: {
    top: boolean
    left: boolean
    title?: string
    className?: string
    children: ReactNode
}) {
    // Flush to the corner: round only the corner facing into the card. The two edges (and the outer
    // corner) stay square so the tag sits flush against the card border.
    const innerRound = top ? (left ? 'rounded-br' : 'rounded-bl') : left ? 'rounded-tr' : 'rounded-tl'
    return (
        <span
            title={title}
            className={cn(
                'absolute flex items-center gap-0.5 bg-black/55 px-1 text-[10px] leading-4 text-white',
                top ? 'top-0' : 'bottom-0',
                left ? 'left-0' : 'right-0',
                innerRound,
                className,
            )}
        >
            {children}
        </span>
    )
}

interface PhotoCardProps {
    item: PictureListItem
    /** Baseline row height (px); flex-basis is derived from it and the aspect ratio. */
    rowHeight: number
    selected: boolean
    /** Touch multi-select mode is active — show the selection circle on every card. */
    multiSelect: boolean
    /** Trash-only view — show the purge countdown overlay on owned trashed pictures. */
    showPurgeCountdown?: boolean
    /** Owner's `trash_retention_days` (for the purge deadline); defaults to 30. */
    retentionDays?: number
    /** Reference instant under a `time_near` sort — drives the per-tile time-delta badge (feature 29). */
    proximityRefTime?: string | null
    onSelect: (event: MouseEvent) => void
    /** Long-press (touch) on the card — enters/extends multi-select. */
    onLongPress: () => void
    onOpen: () => void
}

/** How long a touch must be held (ms) before it counts as a long-press. */
const LONG_PRESS_MS = 450
/** Movement (px) beyond which a press is treated as a scroll and cancelled. */
const MOVE_CANCEL_PX = 10

/**
 * One justified-grid cell. Following the reference CSS: flex-basis/flex-grow are
 * set from the picture's aspect ratio (basis = rowHeight × ratio), and the cell
 * carries the picture's `aspect-ratio` so flexbox derives a uniform row height
 * while preserving each picture's shape — no cropping, minimal JS.
 */
export const PhotoCard = memo(function PhotoCard({
                                                     item,
                                                     rowHeight,
                                                     selected,
                                                     multiSelect,
                                                     showPurgeCountdown,
                                                     retentionDays,
                                                     proximityRefTime,
                                                     onSelect,
                                                     onLongPress,
                                                     onOpen
                                                 }: PhotoCardProps) {
    // Long-press detection (touch/pen only — desktop uses modifier-click). A held touch
    // that hasn't moved past the threshold enters multi-select mode; the long-press also
    // suppresses the synthetic click that follows the pointer release.
    const pressTimer = useRef<number | undefined>(undefined)
    const pressStart = useRef<{ x: number; y: number } | null>(null)
    const longPressed = useRef(false)
    // On mobile, full-screen view is reached from the sidebar preview (a single tap opens the
    // sidebar) — so the grid has no double-tap-to-open, which previously opened both at once.
    const isMobile = useIsMobile()

    const cancelPress = () => {
        if (pressTimer.current !== undefined) {
            clearTimeout(pressTimer.current)
            pressTimer.current = undefined
        }
        pressStart.current = null
    }

    const onPointerDown = (e: PointerEvent) => {
        if (e.pointerType === 'mouse') return
        longPressed.current = false
        pressStart.current = {x: e.clientX, y: e.clientY}
        pressTimer.current = window.setTimeout(() => {
            longPressed.current = true
            pressStart.current = null
            navigator.vibrate?.(10)
            onLongPress()
        }, LONG_PRESS_MS)
    }

    const onPointerMove = (e: PointerEvent) => {
        if (!pressStart.current) return
        if (Math.hypot(e.clientX - pressStart.current.x, e.clientY - pressStart.current.y) > MOVE_CANCEL_PX) {
            cancelPress()
        }
    }

    const handleClick = (e: MouseEvent) => {
        // Swallow the click synthesised after a long-press so it doesn't immediately toggle back off.
        if (longPressed.current) {
            longPressed.current = false
            e.stopPropagation()
            return
        }
        onSelect(e)
    }

    // Lay the cell out at the picture's *display* orientation (dimensions are
    // transposed for 90°/270° rotations) so the justified grid stays correct.
    const {width: dispW, height: dispH} = displayDimensions(item.width, item.height, item.orientation)
    const ratio = dispW && dispH ? dispW / dispH : 1
    const basis = rowHeight * ratio
    // Rotate the blurhash placeholder the same way as the thumbnail so it lines up behind it.
    const blurhash = orientedCoverStyle(item.orientation, item.width, item.height)

    const trashed = !!item.deleted_at
    const ownerDeleted = !item.owned && !!item.owner_deleted_at
    // Cross-instance owner whose backend was unreachable at presign time (§3.2) — a distinct
    // "owner offline" tile, not the generic no-thumbnail file icon.
    const ownerOffline = !item.owned && !item.owner_reachable
    // Purge countdown (trash-only view, owned trashed pictures): owner rows carry no `owner_purge_at`,
    // so derive it as `deleted_at + retention`. Received trash is local-only (never purged) — no badge.
    const purgeAt = showPurgeCountdown && trashed && item.owned
        ? ownedPurgeAt(item.deleted_at, retentionDays ?? 30)
        : null
    const purgeCountdown = purgeAt ? countdown(purgeAt) : null
    // Play badge only over a real video frame thumbnail — never over the fallback file-type icon.
    const showPlayBadge = isVideoMime(item.mime_type) && !!item.thumbnail_url
    // Proximity badges (feature 29 §6), at most one active at a time. Geo distance comes from the
    // backend (`distance_m`); the time delta is computed client-side from the reference instant
    // (both timestamps are naive — `new Date()` shifts both by the same offset, so the delta holds).
    const distanceLabel =
        item.distance_m == null
            ? null
            : item.distance_m < 1000
                ? `${Math.round(item.distance_m)}m`
                : `${(item.distance_m / 1000).toFixed(item.distance_m < 10000 ? 1 : 0)}km`
    const timeDeltaLabel =
        proximityRefTime && item.captured_at
            ? formatDuration(new Date(item.captured_at).getTime() - new Date(proximityRefTime).getTime())
            : null

    // Once the thumbnail loads, fade the blurhash out so transparent (PNG) areas reveal the
    // checkerboard backdrop rather than the blurry placeholder.
    const [loaded, setLoaded] = useState(false)

    // Presign auto-refresh (§10): an expired/403 thumbnail re-presigns a fresh URL in place rather
    // than showing a broken image (fixes stale thumbnails after a backgrounded tab resumes).
    const [refreshedThumb, setRefreshedThumb] = useState<string | null>(null)
    const thumbSrc = refreshedThumb ?? item.thumbnail_url
    const onThumbError = usePresignRefresh(() => {
        void getPictureUrl(item.id, variantForSize(rowHeight))
            .then((r) => r.url && setRefreshedThumb(r.url))
            .catch(() => undefined)
    })

    // Retry the owner-offline tile by re-fetching the list (re-presigns against the peer).
    const queryClient = useQueryClient()
    const retryOwner = (e: MouseEvent) => {
        e.stopPropagation()
        void queryClient.invalidateQueries({queryKey: ['pictures']})
    }

    return (
        <li
            style={{
                flexBasis: `${basis}px`,
                flexGrow: basis,
                aspectRatio: `${dispW ?? 1} / ${dispH ?? 1}`,
            }}
            className={cn(
                'group relative cursor-pointer overflow-hidden rounded-[3px] bg-checkerboard',
                selected && 'ring-2 ring-primary ring-offset-2 ring-offset-background',
                trashed && 'opacity-60',
            )}
            onClick={handleClick}
            onDoubleClick={isMobile ? undefined : onOpen}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={cancelPress}
            onPointerCancel={cancelPress}
            onContextMenu={(e) => e.preventDefault()}
        >
            {item.blurhash && (
                <Blurhash
                    hash={item.blurhash}
                    className={cn(blurhash.className, 'transition-opacity duration-200', loaded && 'opacity-0')}
                    style={blurhash.style}
                />
            )}

            {thumbSrc ? (
                <OrientedImage
                    src={thumbSrc}
                    alt={item.filename ?? ''}
                    orientation={item.orientation}
                    width={item.width}
                    height={item.height}
                    className={cn('transition-opacity duration-200', loaded ? 'opacity-100' : 'opacity-0')}
                    onLoad={() => {
                        setLoaded(true)
                        // Record for reuse by the carousel/lightbox (browser already has these bytes).
                        recordImage(item.id, variantForSize(rowHeight), thumbSrc, true)
                    }}
                    onError={onThumbError}
                />
            ) : ownerOffline ? (
                // The owner's instance was unreachable at presign time — a distinct offline tile with
                // a retry affordance (§3.2/§13), not the generic no-thumbnail file icon.
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 bg-muted/40 p-2 text-center text-muted-foreground">
                    <CloudOff className="h-[30%] w-[30%] max-h-12 max-w-12 opacity-60"/>
                    <span className="text-[10px] leading-tight">Owner offline</span>
                    <button
                        onClick={retryOwner}
                        className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:bg-black/10 hover:text-foreground dark:hover:bg-white/10"
                        title="Retry — the owner's instance may be back"
                    >
                        <RefreshCw className="h-2.5 w-2.5"/> Retry
                    </button>
                </div>
            ) : (
                // No thumbnail (pending, or a non-thumbnailable format like a PDF) — file-type icon.
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 p-2 text-center text-muted-foreground">
                    <FileTypeIcon mime={item.mime_type} filename={item.filename} className="h-[38%] w-[38%] max-h-16 max-w-16 opacity-70"/>
                    {item.filename && <span className="max-w-full truncate text-[10px] leading-tight">{item.filename}</span>}
                </div>
            )}

            {showPlayBadge && <PlayBadge size="sm"/>}

            <div
                className={cn(
                    'absolute left-2 top-2 flex h-5 w-5 items-center justify-center rounded-full border border-white/70 bg-black/40 text-white opacity-0 transition-opacity group-hover:opacity-100',
                    multiSelect && 'opacity-100',
                    selected && 'border-primary bg-primary text-primary-foreground opacity-100',
                )}
            >
                {selected && <Check className="h-3.5 w-3.5"/>}
            </div>

            {!item.owned && item.owner_username && (
                <CornerLabel
                    top={false}
                    left
                    className={ownerDeleted ? 'bg-destructive/85' : undefined}
                    title={ownerDeleted ? `Owner deleted this. ${countdown(item.owner_purge_at)}` : undefined}
                >
                    {ownerDeleted && <AlertTriangle className="h-2.5 w-2.5"/>}
                    @{item.owner_username}
                </CornerLabel>
            )}

            {trashed && (
                <CornerLabel top left={false} title="In trash" className="justify-center py-0.5">
                    <Trash2 className="h-3 w-3"/>
                </CornerLabel>
            )}

            {purgeCountdown && (
                <CornerLabel top={false} left title={`Permanently deleted ${deadlineLabel(purgeAt)}`} className="gap-1 bg-destructive/85 font-medium">
                    <Clock className="h-2.5 w-2.5 shrink-0"/>
                    <span className="truncate">{purgeCountdown}</span>
                </CornerLabel>
            )}

            {distanceLabel && (
                <CornerLabel top={false} left={false} title="Distance from the reference point">
                    <MapPin className="h-2.5 w-2.5 shrink-0"/>
                    {distanceLabel}
                </CornerLabel>
            )}

            {timeDeltaLabel && (
                <CornerLabel top={false} left={false} title="Time from the reference photo">
                    <Clock className="h-2.5 w-2.5 shrink-0"/>
                    {timeDeltaLabel}
                </CornerLabel>
            )}
        </li>
    )
})
