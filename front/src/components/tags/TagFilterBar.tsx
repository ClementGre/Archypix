// A breadcrumb-style bar (mirrors the hierarchy breadcrumb) summarising the active flat-gallery tag
// filter: included / included-exactly / excluded tags, each with inline controls (switch an include
// to "exactly" and back, remove) plus a clear-all. Rendered at the top of the centre grid.

import {Ban, Equal, Hash, ListFilter, X} from 'lucide-react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {cn, TagPath} from '@/lib/utils'

function Chip({
                  path,
                  kind,
                  onSwitch,
                  switchTo,
                  onRemove,
              }: {
    path: string
    kind: 'inc' | 'exa' | 'exc'
    /** Toggle this tag's mode (include ↔ exact); absent for exclude. */
    onSwitch?: () => void
    switchTo?: 'inc' | 'exa'
    onRemove: () => void
}) {
    const Icon = kind === 'exa' ? Equal : kind === 'exc' ? Ban : Hash
    return (
        <span
            className={cn(
                'flex max-w-[16rem] items-center gap-1 rounded-full border px-2 py-0.5 text-xs',
                kind === 'exc' ? 'border-destructive/40 text-destructive' : 'border-primary/40 text-primary',
            )}
        >
            <Icon className="h-3 w-3 shrink-0"/>
            <span className="truncate" title={TagPath.toDisplay(path)}>{TagPath.toDisplay(path)}</span>
            {onSwitch && (
                <Tooltip delayDuration={300}>
                    <TooltipTrigger asChild>
                        <button onClick={onSwitch} className="shrink-0 rounded hover:bg-foreground/10" aria-label="Switch match mode">
                            {switchTo === 'exa' ? <Equal className="h-3 w-3"/> : <Hash className="h-3 w-3"/>}
                        </button>
                    </TooltipTrigger>
                    <TooltipContent className="text-xs">
                        {switchTo === 'exa' ? 'Match exactly (no sub-tags)' : 'Include sub-tags too'}
                    </TooltipContent>
                </Tooltip>
            )}
            <button onClick={onRemove} aria-label="Remove filter" className="shrink-0 rounded hover:bg-foreground/10">
                <X className="h-3 w-3"/>
            </button>
        </span>
    )
}

export function TagFilterBar() {
    const {params, update} = useGalleryParams()
    const {tag, include, exact, exclude} = params

    const active = !!tag || include.length > 0 || exact.length > 0 || exclude.length > 0
    if (!active) return null

    const without = (arr: string[], p: string) => arr.filter((x) => x !== p)
    // All includes (the primary `tag` plus the extra include set) render as one group.
    const includes = [...(tag ? [tag] : []), ...include]

    const remove = (p: string) =>
        update({
            tag: tag === p ? null : tag,
            include: without(include, p),
            exact: without(exact, p),
            exclude: without(exclude, p),
        })
    // Include → exact: drop from tag/include, add to exact.
    const toExact = (p: string) =>
        update({
            tag: tag === p ? null : tag,
            include: without(include, p),
            exact: [...new Set([...exact, p])],
        })
    // Exact → include: drop from exact, add to the include set.
    const toInclude = (p: string) =>
        update({exact: without(exact, p), include: [...new Set([...include, p])]})
    const clearAll = () => update({tag: null, include: [], exact: [], exclude: []})

    return (
        <div className="flex flex-wrap items-center gap-1.5 text-sm">
            <ListFilter className="mr-0.5 h-4 w-4 shrink-0 text-muted-foreground"/>
            {includes.map((p) => (
                <Chip key={`inc:${p}`} path={p} kind="inc" switchTo="exa" onSwitch={() => toExact(p)} onRemove={() => remove(p)}/>
            ))}
            {exact.map((p) => (
                <Chip key={`exa:${p}`} path={p} kind="exa" switchTo="inc" onSwitch={() => toInclude(p)} onRemove={() => remove(p)}/>
            ))}
            {exclude.map((p) => (
                <Chip key={`exc:${p}`} path={p} kind="exc" onRemove={() => remove(p)}/>
            ))}
            <button
                onClick={clearAll}
                className="ml-1 rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
            >
                Clear
            </button>
        </div>
    )
}
