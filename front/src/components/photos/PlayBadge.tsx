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
export function PlayBadge({hover = false, className}: { hover?: boolean; className?: string }) {
    return (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <span
                className={cn(
                    'flex h-12 w-12 items-center justify-center rounded-full bg-black/55 text-white shadow-sm ring-1 ring-white/30',
                    hover && 'transition-transform group-hover:scale-110',
                    className,
                )}
            >
                <Play className="h-6 w-6 translate-x-[1px] fill-current"/>
            </span>
        </div>
    )
}
