import {memo, type MouseEvent, type PointerEvent, useRef, useState} from 'react'
import {AlertTriangle, Check, Trash2} from 'lucide-react'
import type {PictureListItem} from '@/lib/types'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {cn} from '@/lib/utils'
import {countdown} from '@/lib/trash'
import {Blurhash} from './Blurhash'
import {FileTypeIcon} from './FileTypeIcon'
import {displayDimensions, orientedCoverStyle, OrientedImage} from './OrientedImage'

interface PhotoCardProps {
    item: PictureListItem
    /** Baseline row height (px); flex-basis is derived from it and the aspect ratio. */
    rowHeight: number
    selected: boolean
    /** Touch multi-select mode is active — show the selection circle on every card. */
    multiSelect: boolean
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
export const PhotoCard = memo(function PhotoCard({item, rowHeight, selected, multiSelect, onSelect, onLongPress, onOpen}: PhotoCardProps) {
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

    // Once the thumbnail loads, fade the blurhash out so transparent (PNG) areas reveal the
    // checkerboard backdrop rather than the blurry placeholder.
    const [loaded, setLoaded] = useState(false)

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

            {item.thumbnail_url ? (
                <OrientedImage
                    src={item.thumbnail_url}
                    alt={item.filename ?? ''}
                    orientation={item.orientation}
                    width={item.width}
                    height={item.height}
                    className={cn('transition-opacity duration-200', loaded ? 'opacity-100' : 'opacity-0')}
                    onLoad={() => setLoaded(true)}
                />
            ) : (
                // No thumbnail (pending, or a non-thumbnailable format like a PDF) — file-type icon.
                <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 p-2 text-center text-muted-foreground">
                    <FileTypeIcon filename={item.filename} className="h-[38%] w-[38%] max-h-16 max-w-16 opacity-70"/>
                    {item.filename && <span className="max-w-full truncate text-[10px] leading-tight">{item.filename}</span>}
                </div>
            )}

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
                <span
                    className={cn(
                        'absolute bottom-1 left-1 flex items-center gap-0.5 rounded px-1 text-[10px] leading-4 text-white',
                        ownerDeleted ? 'bg-destructive/85' : 'bg-black/55',
                    )}
                    title={
                        ownerDeleted
                            ? `Owner deleted this — disappears ${countdown(item.owner_purge_at) || 'soon'}`
                            : undefined
                    }
                >
                    {ownerDeleted && <AlertTriangle className="h-2.5 w-2.5"/>}
                    @{item.owner_username}
                </span>
            )}

            {trashed && (
                <span
                    className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded-full bg-black/55 text-white"
                    title="In trash"
                >
                    <Trash2 className="h-3 w-3"/>
                </span>
            )}
        </li>
    )
})
