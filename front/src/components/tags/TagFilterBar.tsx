// A breadcrumb-style bar (mirrors the hierarchy breadcrumb) summarising the active tag filter: the
// view root plus the cross-cutting include / exclude sets, each removable, plus a clear-all
// (feature 35 §7). The include↔exact switch is gone — *Direct only* in the **View** dropdown covers
// the real use.

import {Ban, Hash, X} from 'lucide-react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useTagTree} from '@/hooks/useTags'
import {display} from '@/lib/tagTree'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {cn, TagPath} from '@/lib/utils'

function Chip({
                  path,
                  name,
                  color,
                  kind,
                  onRemove,
              }: {
    path: string
    /** Display name; the path stays in the tooltip (feature 34 §5). */
    name: string
    /** The tag's own colour, so a chip reads as the same tag as its tree row. */
    color: string | null
    kind: 'inc' | 'exc'
    onRemove: () => void
}) {
    const Icon = kind === 'exc' ? Ban : Hash
    return (
        <Tooltip delayDuration={400}>
            <TooltipTrigger asChild>
                <span
                    className={cn(
                        'flex max-w-[16rem] items-center gap-1 rounded-full border px-2 py-0.5 text-xs',
                        kind === 'exc' ? 'border-destructive/40 text-destructive' : 'border-primary/40 text-primary',
                    )}
                    style={color && kind !== 'exc' ? {borderColor: `${color}99`, color} : undefined}
                >
                    <Icon className="h-3 w-3 shrink-0"/>
                    <span className="truncate">{name}</span>
                    <button onClick={onRemove} aria-label="Remove filter"
                            className="shrink-0 rounded hover:bg-foreground/10">
                        <X className="h-3 w-3"/>
                    </button>
                </span>
            </TooltipTrigger>
            <TooltipContent className="text-xs">{TagPath.toDisplay(path)}</TooltipContent>
        </Tooltip>
    )
}

export function TagFilterBar() {
    const {params, update} = useGalleryParams()
    const {metaByPath} = useTagTree()
    const nameOf = (p: string) => display(p, metaByPath.get(p))
    const colorOf = (p: string) => metaByPath.get(p)?.color ?? null
    const {tag, include, exclude} = params

    const active = !!tag || include.length > 0 || exclude.length > 0
    if (!active) return null

    const without = (arr: string[], p: string) => arr.filter((x) => x !== p)
    // The view root and the cross-cutting include set render as one group.
    const includes = [...(tag ? [tag] : []), ...include]

    const remove = (p: string) =>
        update({
            tag: tag === p ? null : tag,
            include: without(include, p),
            exclude: without(exclude, p),
        })
    const clearAll = () => update({tag: null, include: [], exclude: []})

    const tot_length = includes.length + exclude.length

    return (
        <div className="flex flex-wrap items-center gap-1.5 text-sm">
            {includes.map((p) => (
                <Chip key={`inc:${p}`} path={p} name={nameOf(p)} color={colorOf(p)} kind="inc"
                      onRemove={() => remove(p)}/>
            ))}
            {exclude.map((p) => (
                <Chip key={`exc:${p}`} path={p} name={nameOf(p)} color={colorOf(p)} kind="exc"
                      onRemove={() => remove(p)}/>
            ))}
            {tot_length > 1 && (
                <button
                    onClick={clearAll}
                    className="ml-1 rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
                >
                    Clear
                </button>
            )}
        </div>
    )
}
