import {Play} from 'lucide-react'
import {cn} from '@/lib/utils'

/**
 * Centred play badge overlaid on a video's frame thumbnail/poster. Shared by the grid (`PhotoCard`)
 * and the details pane so both show the exact same affordance. Only render it when a thumbnail
 * exists — over the fallback file-type icon it just looks like clutter.
 *
 * `hover` enables a grow-on-hover effect: pass it only where the poster is actually clickable to play
 * (the details pane). In the grid, clicking a card selects it — it does not play — so the badge is a
 * static indicator with no hover affordance.
 */
/** Badge / icon sizes per `size`. `lg` is the details poster; `sm`/`xs` suit the grid/carousel. */
const SIZES = {
    xs: {badge: 'h-5 w-5', icon: 'h-2.5 w-2.5'},
    sm: {badge: 'h-7 w-7', icon: 'h-3.5 w-3.5'},
    lg: {badge: 'h-12 w-12', icon: 'h-6 w-6'},
} as const

export function PlayBadge({hover = false, size = 'lg', className}: {
    hover?: boolean
    size?: keyof typeof SIZES
    className?: string
}) {
    const s = SIZES[size]
    return (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <span
                className={cn(
                    'flex items-center justify-center rounded-full bg-black/55 text-white shadow-sm ring-1 ring-white/30',
                    s.badge,
                    hover && 'transition-transform group-hover:scale-110',
                    className,
                )}
            >
                <Play className={cn('translate-x-[0.5px] fill-current', s.icon)}/>
            </span>
        </div>
    )
}
