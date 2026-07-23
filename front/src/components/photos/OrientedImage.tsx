import {useLayoutEffect, useRef, useState} from 'react'
import {cn} from '@/lib/utils'
import {Blurhash} from './Blurhash'

/**
 * Decompose an EXIF orientation value (1–8) into the CSS rotation needed to
 * display the raw stored pixels correctly. Mirrored states (2/4/5/7) are
 * collapsed to their rotation component — the app only ever produces rotation
 * via the rotate buttons, matching the worker's orientation handling.
 */
export function orientationTransform(orientation?: number | null): {
    rotation: 0 | 90 | 180 | 270
    swap: boolean
} {
    switch (orientation ?? 1) {
        case 3:
        case 4:
            return {rotation: 180, swap: false}
        case 5:
        case 6:
            return {rotation: 90, swap: true}
        case 7:
        case 8:
            return {rotation: 270, swap: true}
        default:
            return {rotation: 0, swap: false}
    }
}

/** Display (post-rotation) dimensions for a picture, swapping for 90°/270° orientations. */
export function displayDimensions(
    width: number | null | undefined,
    height: number | null | undefined,
    orientation?: number | null,
): { width: number | null; height: number | null } {
    const {swap} = orientationTransform(orientation)
    if (swap) return {width: height ?? null, height: width ?? null}
    return {width: width ?? null, height: height ?? null}
}

/**
 * Positioning for an element that should *cover* a parent already laid out at
 * the picture's display aspect ratio (see `displayDimensions`). For 90°/270° the
 * element is sized to the parent's transposed box and rotated, so its footprint
 * fills the parent. `max-w-none`/`max-h-none` defeat Tailwind preflight's
 * `img { max-width: 100% }`, which would otherwise clamp the >100% width.
 */
export function orientedCoverStyle(
    orientation: number | null | undefined,
    width: number | null | undefined,
    height: number | null | undefined,
): { className: string; style?: React.CSSProperties } {
    const {rotation, swap} = orientationTransform(orientation)
    if (!swap) {
        return {
            className: 'absolute inset-0 h-full w-full',
            style: rotation ? {transform: `rotate(${rotation}deg)`} : undefined,
        }
    }
    const w = width || 1
    const h = height || 1
    return {
        className: 'absolute left-1/2 top-1/2 max-h-none max-w-none',
        style: {
            width: `${(w / h) * 100}%`,
            height: `${(h / w) * 100}%`,
            transform: `translate(-50%, -50%) rotate(${rotation}deg)`,
        },
    }
}

interface OrientedImageProps {
    src: string
    alt: string
    orientation?: number | null
    /** Raw (pre-orientation) pixel dimensions — used to size 90°/270° rotations. */
    width?: number | null
    height?: number | null
    className?: string
    /** Native lazy/eager loading. Defaults to `lazy` (grid); the lightbox/sidebar pass `eager`. */
    loading?: 'lazy' | 'eager'
    onLoad?: React.ReactEventHandler<HTMLImageElement>
    /** Fires when the image fails to load (e.g. an expired/403 presigned URL — see `usePresignRefresh`). */
    onError?: React.ReactEventHandler<HTMLImageElement>
}

/**
 * A raw (un-oriented) thumbnail rendered at its correct display orientation,
 * sized to *cover* its parent. Must be placed inside a `position: relative`
 * parent whose box already carries the display aspect ratio.
 */
export function OrientedImage({src, alt, orientation, width, height, className, loading = 'lazy', onLoad, onError}: OrientedImageProps) {
    const {className: coverClass, style} = orientedCoverStyle(orientation, width, height)
    return (
        <img
            src={src}
            alt={alt}
            loading={loading}
            onLoad={onLoad}
            onError={onError}
            className={cn(coverClass, 'object-cover', className)}
            style={style}
        />
    )
}

interface OrientedContainImageProps {
    /** The image to show. Omit while the URL is still resolving — the `blurhash` placeholder shows. */
    src?: string
    alt: string
    orientation?: number | null
    /**
     * BlurHash placeholder shown behind the image (in the same sized box) until it loads, then
     * faded out. Also shown on its own while `src` is absent (URL still resolving).
     */
    blurhash?: string | null
    /**
     * A lower-resolution URL already loaded by the browser (e.g. the grid's medium thumbnail),
     * painted behind the main image so a sharp-ish picture shows instantly before `src` loads.
     */
    placeholderSrc?: string | null
    /** Called once the main `src` image finishes loading. */
    onLoad?: () => void
    /** Fires when the main `src` image fails to load (expired/403 presign — see `usePresignRefresh`). */
    onError?: React.ReactEventHandler<HTMLImageElement>
    /** Raw (pre-orientation) pixel dimensions — used to derive the display aspect ratio. */
    width?: number | null
    height?: number | null
    /**
     * When set, the component flows in normal layout and hugs the image height,
     * capped at this many pixels (used for the sidebar preview so landscape
     * pictures get no letterbox margins). When unset, it fills its parent
     * (`absolute inset-0`) and fits within the parent's measured box (lightbox).
     */
    maxHeight?: number
    className?: string
    /** Click handler attached to the sized image box only (not the surrounding centring area). */
    onClick?: React.MouseEventHandler<HTMLDivElement>
    /**
     * The `src` is the untouched original blob, which keeps its EXIF orientation — the browser already
     * auto-rotates it (`image-orientation: from-image`, the default, which works cross-origin). When
     * set, the app's CSS rotation is skipped for the main image only, avoiding a double-rotation. The
     * box is still laid out at the display aspect ratio, and the blurhash / low-res placeholder (raw
     * pixels with no EXIF) keep the app's rotation.
     */
    srcOriented?: boolean
}

/**
 * A raw thumbnail rendered at its correct display orientation, scaled to *fit*
 * (contain) the available space. The available box is measured (pure CSS cannot
 * fit a rotated box into a container of unknown aspect ratio); the image is then
 * placed in an exact display-aspect box so `OrientedImage` covers it without
 * cropping.
 */
export function OrientedContainImage({
                                         src,
                                         alt,
                                         orientation,
                                         blurhash,
                                         placeholderSrc,
                                         width,
                                         height,
                                         maxHeight,
                                         className,
                                         onLoad,
                                         onError,
                                         onClick,
                                         srcOriented,
                                     }: OrientedContainImageProps) {
    const {width: dW, height: dH} = displayDimensions(width, height, orientation)
    const aspect = dW && dH ? dW / dH : 1

    // Track the loaded state per-URL so it flips to false *synchronously* when `src` changes (an
    // effect-based reset leaves one frame where the placeholders are hidden under the new-but-unloaded
    // image — the flicker). `loaded` is derived, so navigating never flashes the blurhash.
    const [loadedSrc, setLoadedSrc] = useState<string | null>(null)
    const loaded = !!src && loadedSrc === src
    const hasPlaceholder = !!placeholderSrc && placeholderSrc !== src
    const cover = orientedCoverStyle(orientation, width, height)

    const ref = useRef<HTMLDivElement>(null)
    // Seed with the element's current width so the first paint is already sized (avoids a flash and a
    // brief mis-size on wide/16:9 previews before the observer's first tick).
    const [avail, setAvail] = useState({w: 0, h: 0})
    // Layout effect (not effect) so the box is measured *before* paint — otherwise the first frame
    // after (re)mount has avail=0 and renders nothing, which reads as a flicker when navigating.
    useLayoutEffect(() => {
        const el = ref.current
        if (!el) return
        setAvail({w: el.clientWidth, h: el.clientHeight})
        const ro = new ResizeObserver((entries) => {
            const r = entries[0].contentRect
            setAvail({w: r.width, h: r.height})
        })
        ro.observe(el)
        return () => ro.disconnect()
    }, [])

    const flow = maxHeight != null
    const availW = avail.w
    const availH = flow ? maxHeight! : avail.h

    let boxW = 0
    let boxH = 0
    if (availW > 0 && availH > 0) {
        if (availW / availH > aspect) {
            boxH = availH
            boxW = availH * aspect
        } else {
            boxW = availW
            boxH = availW / aspect
        }
        // Never exceed the available box in either axis (guards against a stale measurement
        // letting a wide 16:9 preview overflow its container).
        if (boxW > availW) {
            boxW = availW
            boxH = availW / aspect
        }
        if (boxH > availH) {
            boxH = availH
            boxW = availH * aspect
        }
    }

    return (
        <div
            ref={ref}
            className={cn(flow ? 'flex w-full justify-center overflow-hidden' : 'absolute inset-0 flex items-center justify-center', className)}
            style={flow ? {height: boxH || undefined} : undefined}
        >
            {boxW > 0 && (
                <div className="relative max-w-full overflow-hidden bg-checkerboard" style={{width: boxW, height: boxH}} onClick={onClick}>
                    {blurhash && (
                        <Blurhash
                            hash={blurhash}
                            className={cn(cover.className, 'transition-opacity duration-300', loaded && 'opacity-0')}
                            style={cover.style}
                        />
                    )}
                    {/* Lower-res already-loaded picture, painted behind `src` (eager so a browser-cached
                        thumbnail paints instantly, covering the blurhash — no fade, the main image
                        simply covers it once loaded). Kept mounted until the main image has loaded. */}
                    {hasPlaceholder && !loaded && (
                        <OrientedImage src={placeholderSrc!} alt={alt} orientation={orientation} width={width} height={height} loading="eager"/>
                    )}
                    {src && (
                        <OrientedImage
                            key={src}
                            src={src}
                            alt={alt}
                            // The original blob is auto-rotated by the browser from its EXIF — passing 1
                            // skips the app's CSS rotation so it isn't rotated twice.
                            orientation={srcOriented ? 1 : orientation}
                            width={width}
                            height={height}
                            loading="eager"
                            // Gate visibility on load, but **snap** (no opacity transition): a cross-fade
                            // momentarily shows the dark backdrop through the still-transparent image (the
                            // blurhash/placeholder behind it fades/unmounts in lockstep), which reads as a
                            // dark re-flash. A hard cut from the low-res placeholder to the sharp image has none.
                            className={blurhash || hasPlaceholder ? (loaded ? 'opacity-100' : 'opacity-0') : undefined}
                            onLoad={() => {
                                setLoadedSrc(src)
                                onLoad?.()
                            }}
                            onError={onError}
                        />
                    )}
                </div>
            )}
        </div>
    )
}
