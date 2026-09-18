// The collapsed subtag card (feature 35 §5): a photo-sized tile in the flex flow carrying the
// cover, display name, count and date range. Costs zero requests unless the tag has an explicit
// cover — there is no auto-derived cover (34 §2), so the fallback is a tinted plate.

import {type DragEvent, useState} from 'react'
import {ArrowUpRight, ChevronRight, Hash} from 'lucide-react'
import {countsFor, effectiveDate, type TagNode, type TrashView} from '@/lib/tagTree'
import {cn, TagPath} from '@/lib/utils'
import {parseNaive} from '@/lib/fixDate'
import {useTagDragStore} from '@/stores/tagDrag'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {OrientedCoverImage} from '@/components/photos/CoverThumb'

/** `2026-08-01` + `2026-08-14` → "Aug 1 – 14"; a single day collapses to one label. A tag's range is
 *  MIN/MAX(captured_at) — wall-clock, like the grouping buckets, so it is never shifted to local. */
export function formatRange(from: string | null, to: string | null): string | null {
    if (!from && !to) return null
    const fmt = (iso: string, withYear: boolean) =>
        parseNaive(iso.replace(' ', 'T')).date.toLocaleDateString(undefined, {
            month: 'short',
            day: 'numeric',
            ...(withYear ? {year: 'numeric'} : {}),
        })
    if (!from || !to) return fmt((from ?? to)!, true)
    const sameDay = from.slice(0, 10) === to.slice(0, 10)
    if (sameDay) return fmt(from, true)
    const sameYear = from.slice(0, 4) === to.slice(0, 4)
    return `${fmt(from, !sameYear)} – ${fmt(to, true)}`
}

export function SubtagBlock({
                                node,
                                trash,
                                rowHeight,
                                onToggle,
                                onOpen,
                                onDrop,
                                /** Counts come from the unfiltered tag payload, so they would be wrong here (§5). */
                                hideCount,
                            }: {
    node: TagNode
    trash: TrashView
    rowHeight: number
    onToggle: () => void
    /** Make this tag the view root — browsing *into* it rather than expanding it in place. */
    onOpen: () => void
    onDrop: (node: TagNode) => void
    hideCount: boolean
}) {
    const counts = countsFor(node, trash)
    const range = formatRange(effectiveDate(node, 'date_from', trash), effectiveDate(node, 'date_to', trash))
    const dragging = useTagDragStore((s) => s.selection !== null)
    const [over, setOver] = useState(false)
    // `SharedToMe.*` is structural — photos can never be dropped onto it (34 §9).
    const droppable = dragging && !TagPath.isProtected(node.path)
    const accent = node.meta?.color ?? undefined

    return (
        <li
            className="group/block relative"
            // Width comes from the grid track, so a short last row keeps the same tile size as a full
            // one — unlike photos, a tag has no aspect ratio, so blocks should read as a row of equals.
            style={{height: rowHeight}}
            onDragOver={(e: DragEvent) => {
                if (!dragging) return
                e.preventDefault()
                e.dataTransfer.dropEffect = droppable ? 'copy' : 'none'
                setOver(droppable)
            }}
            onDragLeave={() => setOver(false)}
            onDrop={(e: DragEvent) => {
                setOver(false)
                if (!droppable) return
                e.preventDefault()
                onDrop(node)
            }}
        >
            <Tooltip delayDuration={400}>
                <TooltipTrigger asChild>
                    <button
                        onClick={onToggle}
                        aria-expanded={false}
                        className={cn(
                            'relative flex h-full w-full flex-col justify-end overflow-hidden rounded-[3px] border bg-muted/60 p-2 text-left',
                            'hover:border-primary/60',
                            over && 'ring-2 ring-primary ring-inset',
                            dragging && !droppable && 'cursor-not-allowed opacity-50',
                        )}
                        style={accent ? {borderColor: `${accent}80`} : undefined}
                    >
                        {node.meta?.cover_picture_id && (
                            <OrientedCoverImage pictureId={node.meta.cover_picture_id} alt={node.name}
                                                className="absolute inset-0 h-full w-full"/>
                        )}
                        <span className="absolute inset-0 bg-gradient-to-t from-black/75 via-black/25 to-transparent"/>
                        <span className="relative flex min-w-0 items-center gap-1 text-xs font-medium text-white">
                            {accent ? (
                                <span className="h-2 w-2 shrink-0 rounded-full"
                                      style={{backgroundColor: accent}} aria-hidden/>
                            ) : (
                                <Hash className="h-3 w-3 shrink-0 opacity-70"/>
                            )}
                            <span className="truncate">{node.name}</span>
                            <ChevronRight className="ml-auto h-3.5 w-3.5 shrink-0"/>
                        </span>
                        <span className="relative truncate text-[10px] text-white/75">
                            {range}
                            {range && !hideCount && ' · '}
                            {!hideCount && `${counts.count}`}
                        </span>
                    </button>
                </TooltipTrigger>
                <TooltipContent className="text-xs">{TagPath.toDisplay(node.path)}</TooltipContent>
            </Tooltip>
            {/* Expanding keeps you here; this browses *into* the tag, making it the view root. */}
            <OpenTagButton onOpen={onOpen} name={node.name}/>
        </li>
    )
}

/** The "open in its own view" affordance, shared by the collapsed tile and the expanded header. */
export function OpenTagButton({onOpen, name, inline, className}: {
    onOpen: () => void
    name: string
    /** Sits in a header row rather than floating over a cover: in flow, centred, always visible. */
    inline?: boolean
    className?: string
}) {
    return (
        <Tooltip delayDuration={400}>
            <TooltipTrigger asChild>
                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        onOpen()
                    }}
                    aria-label={`Open ${name}`}
                    className={cn(
                        'flex items-center justify-center rounded p-1 transition-opacity',
                        inline
                            ? 'shrink-0 text-muted-foreground hover:bg-muted hover:text-foreground'
                            : 'absolute right-1 top-1 bg-black/45 text-white/90 opacity-0 hover:bg-black/70 focus-visible:opacity-100 group-hover/block:opacity-100 max-md:opacity-100',
                        className,
                    )}
                >
                    <ArrowUpRight className="h-3.5 w-3.5"/>
                </button>
            </TooltipTrigger>
            <TooltipContent className="text-xs">Open in its own view</TooltipContent>
        </Tooltip>
    )
}
