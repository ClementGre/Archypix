import {useEffect, useRef} from 'react'
import {decode} from 'blurhash'

/** Renders a BlurHash string to a small canvas, used as a thumbnail placeholder. */
export function Blurhash({
                             hash,
                             width = 32,
                             height = 32,
                             punch = 1,
                             className,
                           style,
                         }: {
    hash: string
    width?: number
    height?: number
    punch?: number
    className?: string
  style?: React.CSSProperties
}) {
    const ref = useRef<HTMLCanvasElement>(null)

    useEffect(() => {
        const canvas = ref.current
        if (!canvas) return
        try {
            const pixels = decode(hash, width, height, punch)
            const ctx = canvas.getContext('2d')
            if (!ctx) return
            const image = ctx.createImageData(width, height)
            image.data.set(pixels)
            ctx.putImageData(image, 0, 0)
        } catch {
            // invalid hash — leave the canvas blank
        }
    }, [hash, width, height, punch])

  return <canvas ref={ref} width={width} height={height} className={className} style={style}/>
}
