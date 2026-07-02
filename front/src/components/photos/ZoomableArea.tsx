import {type PointerEvent as ReactPointerEvent, type ReactNode, useCallback, useEffect, useRef, useState} from 'react'
import {cn} from '@/lib/utils'

const MIN_SCALE = 1
const MAX_SCALE = 8

const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v))

/**
 * Zoom & pan container for the lightbox image. Ctrl/⌘ + mouse wheel (or trackpad pinch, which the
 * browser reports as a ctrl-wheel) zooms toward the cursor; once zoomed in, dragging with the mouse
 * pans. Resets whenever `resetKey` changes (i.e. the picture changes). Applies to still images only.
 */
export function ZoomableArea({resetKey, children}: { resetKey: string; children: ReactNode }) {
    const ref = useRef<HTMLDivElement>(null)
    const [t, setT] = useState({scale: 1, x: 0, y: 0})
    const drag = useRef<{ startX: number; startY: number; x: number; y: number } | null>(null)

    useEffect(() => setT({scale: 1, x: 0, y: 0}), [resetKey])

    // Wheel-zoom via a non-passive native listener (React's onWheel is passive, so preventDefault —
    // needed to stop the browser's own ctrl-wheel page zoom — would be ignored).
    useEffect(() => {
        const el = ref.current
        if (!el) return
        const onWheel = (e: WheelEvent) => {
            if (!e.ctrlKey && !e.metaKey) return
            e.preventDefault()
            const rect = el.getBoundingClientRect()
            const cx = e.clientX - rect.left - rect.width / 2
            const cy = e.clientY - rect.top - rect.height / 2
            setT((prev) => {
                const scale = clamp(prev.scale * Math.exp(-e.deltaY * 0.002), MIN_SCALE, MAX_SCALE)
                if (scale === 1) return {scale: 1, x: 0, y: 0}
                const k = scale / prev.scale
                return {scale, x: cx - (cx - prev.x) * k, y: cy - (cy - prev.y) * k}
            })
        }
        el.addEventListener('wheel', onWheel, {passive: false})
        return () => el.removeEventListener('wheel', onWheel)
    }, [])

    const onPointerDown = useCallback((e: ReactPointerEvent) => {
        if (t.scale <= 1 || e.button !== 0) return
        e.preventDefault()
        e.stopPropagation()
        drag.current = {startX: e.clientX, startY: e.clientY, x: t.x, y: t.y}
        ;(e.target as HTMLElement).setPointerCapture?.(e.pointerId)
    }, [t])

    const onPointerMove = useCallback((e: ReactPointerEvent) => {
        if (!drag.current) return
        setT((prev) => ({...prev, x: drag.current!.x + (e.clientX - drag.current!.startX), y: drag.current!.y + (e.clientY - drag.current!.startY)}))
    }, [])

    const endDrag = useCallback(() => {
        drag.current = null
    }, [])

    const zoomed = t.scale > 1
    return (
        <div
            ref={ref}
            className={cn('absolute inset-0 flex items-center justify-center', zoomed && (drag.current ? 'cursor-grabbing' : 'cursor-grab'))}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={endDrag}
            onPointerCancel={endDrag}
            onDoubleClick={(e) => {
                e.stopPropagation()
                setT((prev) => (prev.scale > 1 ? {scale: 1, x: 0, y: 0} : {scale: 2, x: 0, y: 0}))
            }}
        >
            <div
                className="relative flex h-full w-full items-center justify-center"
                style={{transform: `translate(${t.x}px, ${t.y}px) scale(${t.scale})`, transition: drag.current ? 'none' : 'transform 0.08s ease-out'}}
            >
                {children}
            </div>
        </div>
    )
}
