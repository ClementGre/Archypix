// Sort + Group by (feature 35 §4) — **one** two-column dropdown. They merge because the dependency
// is structural: a bucket must be a contiguous run in the sorted order, or the same bucket reappears
// down the page. A dependent second column states that constraint instead of leaving two sibling
// menus free to disagree. Buckets are remembered per sort field (34 §3.1), so switching sort and
// back does not lose the setting.
//
// Proximity sorts (feature 29) are set from a picture's "Find nearby" action rather than chosen
// here, so the menu only surfaces the active one with a one-click clear.

import {ArrowUpDown, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuLabel,
    DropdownMenuRadioGroup,
    DropdownMenuRadioItem,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useTimelineView} from '@/hooks/useTimelineView'
import {kindsFor} from '@/lib/grouping'
import type {GroupingKind, SortField, SortOrder} from '@/lib/types'
import {cn} from '@/lib/utils'

const SORT_FIELDS: { value: SortField; label: string }[] = [
    {value: 'captured_at', label: 'Date taken'},
    {value: 'ingested_at', label: 'Date added'},
    {value: 'updated_at', label: 'Last modified'},
    {value: 'file_size', label: 'File size'},
    {value: 'filename', label: 'Name'},
]

const KIND_LABELS: Record<GroupingKind['kind'], string> = {
    none: 'No grouping',
    year: 'Year',
    quarter: 'Quarter',
    season: 'Season',
    month: 'Month',
    prefix: 'Name prefix',
    magnitude: 'Magnitude',
}

/** Default `prefix` width — `IMG`, `DSC`, `PXL` are all three characters. */
const DEFAULT_PREFIX_CHARS = 3

export function SortMenu() {
    const {params, update} = useGalleryParams()
    const {grouping, pinned, setGrouping} = useTimelineView()
    const isProximity = params.sort === 'time_near' || params.sort === 'geo_near'
    // Hierarchy browse keeps today's flat rendering, so the bucket column drops out (§10.9).
    const bucketing = !pinned && !params.hierarchy
    const kinds = kindsFor(params.sort)

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button
                    variant="outline"
                    size="sm"
                    // A proximity sort is transient URL state, so it stays highlighted with its
                    // one-click clear; a stored grouping is an ordinary preference and is not.
                    className={cn(
                        'gap-1.5 text-xs font-normal',
                        isProximity ? 'border-primary/50 text-primary' : 'text-muted-foreground',
                    )}
                >
                    <ArrowUpDown className="h-3.5 w-3.5"/>
                    <span className="hidden sm:inline">Sort</span>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[19rem]">
                {isProximity && (
                    <>
                        <DropdownMenuLabel>Proximity</DropdownMenuLabel>
                        <DropdownMenuItem
                            className="gap-2 text-primary"
                            onSelect={() => update({sort: 'captured_at', nearTime: null, nearLat: null, nearLng: null})}
                        >
                            <span className="flex-1">
                                {params.sort === 'time_near' ? 'Nearby in time' : 'Nearby in place'}
                            </span>
                            <X className="h-3.5 w-3.5"/>
                        </DropdownMenuItem>
                        <DropdownMenuSeparator/>
                    </>
                )}

                {/* On mobile the two columns stack, the bucket column disclosing under the field (§4). */}
                <div className="flex flex-col sm:flex-row">
                    <div className="min-w-0 flex-1">
                        <DropdownMenuLabel>Sort by</DropdownMenuLabel>
                        <DropdownMenuRadioGroup
                            value={params.sort}
                            onValueChange={(v) => update({sort: v as SortField})}
                        >
                            {SORT_FIELDS.map((f) => (
                                <DropdownMenuRadioItem key={f.value} value={f.value}>
                                    {f.label}
                                </DropdownMenuRadioItem>
                            ))}
                        </DropdownMenuRadioGroup>
                    </div>

                    {bucketing && (
                        <div className="min-w-0 flex-1 border-t sm:border-l sm:border-t-0">
                            <DropdownMenuLabel>Group by</DropdownMenuLabel>
                            <DropdownMenuRadioGroup
                                value={grouping.kind}
                                onValueChange={(v) =>
                                    setGrouping(
                                        v === 'prefix'
                                            ? {kind: 'prefix', chars: DEFAULT_PREFIX_CHARS}
                                            : {kind: v as Exclude<GroupingKind['kind'], 'prefix'>},
                                    )
                                }
                            >
                                {kinds.map((k) => (
                                    <DropdownMenuRadioItem key={k} value={k}>
                                        {KIND_LABELS[k]}
                                    </DropdownMenuRadioItem>
                                ))}
                            </DropdownMenuRadioGroup>
                            {grouping.kind === 'prefix' && (
                                <div className="flex items-center gap-2 px-2 py-1.5"
                                     onClick={(e) => e.stopPropagation()}>
                                    <label htmlFor="prefix-chars" className="text-[11px] text-muted-foreground">
                                        First
                                    </label>
                                    <input
                                        id="prefix-chars"
                                        type="number"
                                        min={1}
                                        max={16}
                                        value={grouping.chars}
                                        onChange={(e) => {
                                            const n = Math.min(16, Math.max(1, Number(e.target.value) || 1))
                                            setGrouping({kind: 'prefix', chars: n})
                                        }}
                                        className="h-6 w-14 rounded border bg-background px-1 text-xs"
                                    />
                                    <span className="text-[11px] text-muted-foreground">chars</span>
                                </div>
                            )}
                        </div>
                    )}
                </div>

                <DropdownMenuSeparator/>
                <DropdownMenuRadioGroup value={params.order} onValueChange={(v) => update({order: v as SortOrder})}>
                    <DropdownMenuRadioItem value="desc">Descending</DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value="asc">Ascending</DropdownMenuRadioItem>
                </DropdownMenuRadioGroup>

                {pinned && (
                    <p className="px-2 py-1.5 text-[11px] text-muted-foreground">
                        Grouping is off while a fix mode is active — it needs a flat stream to find neighbours.
                    </p>
                )}
            </DropdownMenuContent>
        </DropdownMenu>
    )
}
