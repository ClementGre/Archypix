import {type ReactNode, useCallback, useEffect, useRef} from 'react'
import {SIDEBAR_MAX, SIDEBAR_MIN} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {cn} from '@/lib/utils'

interface SidePanelProps {
    side: 'left' | 'right'
    width: number
    onResize: (width: number) => void
    /** Closed state hides the panel (desktop) or the drawer + backdrop (mobile). */
    open: boolean
    onClose: () => void
    children: ReactNode
}

/**
 * A resizable workspace side panel. On desktop it sits inline with a draggable
 * edge handle (width persisted by the caller); on mobile it becomes an overlay
 * drawer with a backdrop. Resizing is desktop-only.
 */
export function SidePanel({side, width, onResize, open, onClose, children}: SidePanelProps) {
    const isMobile = useIsMobile()
    const startX = useRef(0)
    const startWidth = useRef(0)

    const onPointerMove = useCallback(
        (e: PointerEvent) => {
            const delta = e.clientX - startX.current
            const next = side === 'left' ? startWidth.current + delta : startWidth.current - delta
            onResize(next)
        },
        [side, onResize],
    )

    const onPointerUp = useCallback(() => {
        document.removeEventListener('pointermove', onPointerMove)
        document.removeEventListener('pointerup', onPointerUp)
        document.body.style.userSelect = ''
        document.body.style.cursor = ''
    }, [onPointerMove])

    const onPointerDown = (e: React.PointerEvent) => {
        e.preventDefault()
        startX.current = e.clientX
        startWidth.current = width
        document.body.style.userSelect = 'none'
        document.body.style.cursor = 'col-resize'
        document.addEventListener('pointermove', onPointerMove)
        document.addEventListener('pointerup', onPointerUp)
    }

    useEffect(
        () => () => {
            document.removeEventListener('pointermove', onPointerMove)
            document.removeEventListener('pointerup', onPointerUp)
        },
        [onPointerMove, onPointerUp],
    )

    if (!open) return null

    const borderClass = side === 'left' ? 'border-r' : 'border-l'

    if (isMobile) {
        return (
            <>
                <div className="fixed inset-0 z-40 bg-black/50" onClick={onClose} aria-hidden/>
                <aside
                    className={cn(
                        'fixed bottom-0 top-0 z-50 flex max-w-[85vw] flex-col overflow-hidden bg-card',
                        borderClass,
                        side === 'left' ? 'left-0' : 'right-0',
                    )}
                    style={{width}}
                >
                    {children}
                </aside>
            </>
        )
    }

    const handle = (
        <div
            onPointerDown={onPointerDown}
            onDoubleClick={() => onResize(side === 'left' ? 256 : 288)}
            role="separator"
            aria-orientation="vertical"
            className={cn(
                'absolute top-0 z-10 h-full w-1.5 cursor-col-resize transition-colors hover:bg-primary/40',
                side === 'left' ? 'right-0' : 'left-0',
            )}
        />
    )

    return (
        <aside
            className={cn('relative flex shrink-0 flex-col overflow-hidden bg-card', borderClass)}
            style={{width, minWidth: SIDEBAR_MIN, maxWidth: SIDEBAR_MAX}}
        >
            {children}
            {handle}
        </aside>
    )
}
