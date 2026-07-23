import {type PointerEvent as ReactPointerEvent, type ReactNode, useCallback, useEffect, useRef, useState} from 'react'
import {cn} from '@/lib/utils'

const MIN_SCALE = 1
const MAX_SCALE = 8

// Wheel/pinch zoom feel (the ctrl+wheel path — Chrome/Firefox, which have no gesture events). The
// browser synthesizes a trackpad pinch as a ctrl+wheel with small pixel deltas; a mouse notch is a
// large coarse delta. A single exponential (scale *= e^(-delta·k)) handles both, with the delta
// clamped so a mouse notch stays reasonable while fine pinch deltas pass through untouched.
const WHEEL_ZOOM_K = 0.01
const MAX_WHEEL_DELTA = 40

// Non-standard Safari/WebKit pinch gesture. `scale` is the native trackpad magnification (starts at
// 1 on gesturestart), so we get exactly the macOS Preview feel there.
interface GestureEventLike extends MouseEvent {
    scale: number
    rotation: number
}

const clamp = (v: number, min: number, max: number) => Math.min(max, Math.max(min, v))

/**
 * Zoom & pan container for the lightbox image. Trackpad pinch zooms toward the cursor (native
 * magnification on Safari via gesture events; ctrl-wheel elsewhere); ctrl/⌘ + mouse wheel also zooms;
 * once zoomed in, dragging pans. Resets whenever `resetKey` changes (the picture changes).
 */
export function ZoomableArea({resetKey, children}: { resetKey: string; children: ReactNode }) {
    const ref = useRef<HTMLDivElement>(null)
    const [t, setT] = useState({scale: 1, x: 0, y: 0})
    // Mirror the transform so the native wheel listener (stable across renders) can read the current
    // scale without re-subscribing.
    const tRef = useRef(t)
    tRef.current = t
    const drag = useRef<{ startX: number; startY: number; x: number; y: number } | null>(null)
    // Transition is disabled while actively zooming (so the image tracks the fingers 1:1, like a
    // native app) and re-enabled shortly after, so the double-click zoom still animates.
    const [smooth, setSmooth] = useState(true)
    const smoothTimer = useRef<number>()

    useEffect(() => setT({scale: 1, x: 0, y: 0}), [resetKey])

    const bumpInteracting = useCallback(() => {
        setSmooth(false)
        if (smoothTimer.current) clearTimeout(smoothTimer.current)
        smoothTimer.current = window.setTimeout(() => setSmooth(true), 100)
    }, [])

    // Clamp a pan offset so the (scaled) picture can't be dragged past its own edges — the image box
    // always covers the container on each axis where it's larger, and stays centred where it isn't.
    // The image's *unscaled* box is measured from the rendered <img> (its bounding rect ÷ current
    // scale), so bounds match the actual contained/rotated picture, not the full container.
    const clampPan = useCallback((scale: number, x: number, y: number) => {
        const el = ref.current
        if (!el) return {x, y}
        const img = el.querySelector('img')
        const cur = tRef.current.scale || 1
        let baseW = el.clientWidth
        let baseH = el.clientHeight
        if (img) {
            const r = img.getBoundingClientRect()
            baseW = r.width / cur
            baseH = r.height / cur
        }
        const maxX = Math.max(0, (baseW * scale - el.clientWidth) / 2)
        const maxY = Math.max(0, (baseH * scale - el.clientHeight) / 2)
        return {x: clamp(x, -maxX, maxX), y: clamp(y, -maxY, maxY)}
    }, [])

    // Multiply the current scale by `factor`, keeping the point under (clientX, clientY) fixed.
    const applyZoom = useCallback((factor: number, clientX: number, clientY: number) => {
        const el = ref.current
        if (!el) return
        const rect = el.getBoundingClientRect()
        const cx = clientX - rect.left - rect.width / 2
        const cy = clientY - rect.top - rect.height / 2
        setT((prev) => {
            const scale = clamp(prev.scale * factor, MIN_SCALE, MAX_SCALE)
            if (scale === 1) return {scale: 1, x: 0, y: 0}
            const k = scale / prev.scale
            return {scale, ...clampPan(scale, cx - (cx - prev.x) * k, cy - (cy - prev.y) * k)}
        })
    }, [clampPan])

    // Native listeners (React's onWheel is passive, so preventDefault — needed to stop the browser's
    // own ctrl-wheel / gesture page zoom — would be ignored). Safari's gesture events give the true
    // native trackpad magnification; ctrl-wheel is the Chrome/Firefox fallback.
    useEffect(() => {
        const el = ref.current
        if (!el) return

        const gesturing = {active: false, last: 1}

        const onWheel = (e: WheelEvent) => {
            if (gesturing.active) return // Safari fires gesture events instead — don't double-count
            if (e.ctrlKey || e.metaKey) {
                // Zoom toward the cursor (trackpad pinch on Chrome/Firefox, or ctrl+mouse-wheel).
                e.preventDefault()
                let d = e.deltaY
                if (e.deltaMode === 1) d *= 16 // lines → px (Firefox mouse wheel)
                else if (e.deltaMode === 2) d *= el.clientHeight // pages (rare)
                d = clamp(d, -MAX_WHEEL_DELTA, MAX_WHEEL_DELTA)
                bumpInteracting()
                applyZoom(Math.exp(-d * WHEEL_ZOOM_K), e.clientX, e.clientY)
                return
            }
            // Plain two-finger scroll pans, but only once zoomed in (nothing to pan otherwise).
            if (tRef.current.scale <= 1) return
            e.preventDefault()
            let dx = e.deltaX
            let dy = e.deltaY
            if (e.deltaMode === 1) {
                dx *= 16
                dy *= 16
            }
            bumpInteracting()
            setT((prev) => (prev.scale <= 1 ? prev : {...prev, ...clampPan(prev.scale, prev.x - dx, prev.y - dy)}))
        }

        const onGestureStart = (e: Event) => {
            e.preventDefault()
            gesturing.active = true
            gesturing.last = 1
        }
        const onGestureChange = (e: Event) => {
            const g = e as GestureEventLike
            e.preventDefault()
            const factor = gesturing.last ? g.scale / gesturing.last : 1
            gesturing.last = g.scale
            bumpInteracting()
            applyZoom(factor, g.clientX, g.clientY)
        }
        const onGestureEnd = (e: Event) => {
            e.preventDefault()
            gesturing.active = false
        }

        el.addEventListener('wheel', onWheel, {passive: false})
        // Present only on Safari/WebKit; a no-op elsewhere.
        el.addEventListener('gesturestart', onGestureStart as EventListener)
        el.addEventListener('gesturechange', onGestureChange as EventListener)
        el.addEventListener('gestureend', onGestureEnd as EventListener)
        return () => {
            el.removeEventListener('wheel', onWheel)
            el.removeEventListener('gesturestart', onGestureStart as EventListener)
            el.removeEventListener('gesturechange', onGestureChange as EventListener)
            el.removeEventListener('gestureend', onGestureEnd as EventListener)
        }
    }, [applyZoom, bumpInteracting, clampPan])

    const onPointerDown = useCallback((e: ReactPointerEvent) => {
        if (t.scale <= 1 || e.button !== 0) return
        e.preventDefault()
        e.stopPropagation()
        drag.current = {startX: e.clientX, startY: e.clientY, x: t.x, y: t.y}
        ;(e.target as HTMLElement).setPointerCapture?.(e.pointerId)
    }, [t])

    const onPointerMove = useCallback((e: ReactPointerEvent) => {
        if (!drag.current) return
        setT((prev) => ({...prev, ...clampPan(prev.scale, drag.current!.x + (e.clientX - drag.current!.startX), drag.current!.y + (e.clientY - drag.current!.startY))}))
    }, [clampPan])

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
                setSmooth(true)
                setT((prev) => (prev.scale > 1 ? {scale: 1, x: 0, y: 0} : {scale: 2, x: 0, y: 0}))
            }}
        >
            <div
                className="relative flex h-full w-full items-center justify-center"
                style={{
                    transform: `translate(${t.x}px, ${t.y}px) scale(${t.scale})`,
                    transition: drag.current || !smooth ? 'none' : 'transform 0.12s ease-out'
                }}
            >
                {children}
            </div>
        </div>
    )
}
