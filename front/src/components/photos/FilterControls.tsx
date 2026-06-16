import {useEffect, useState} from 'react'
import {ArrowUpDown, Check, Search, SlidersHorizontal, X} from 'lucide-react'
import {Input} from '@/components/ui/input'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {type Scope, useGalleryParams} from '@/hooks/useGalleryParams'
import {useDebouncedValue} from '@/hooks/useDebouncedValue'
import type {SortField, SortOrder} from '@/lib/types'
import {cn, TagPath} from '@/lib/utils'

const SCOPES: { value: Scope; label: string }[] = [
  {value: 'all', label: 'All photos'},
    {value: 'owned', label: 'Mine'},
  {value: 'shared', label: 'Shared with me'},
]

const SORT_FIELDS: { value: SortField; label: string }[] = [
    {value: 'ingested_at', label: 'Date added'},
    {value: 'captured_at', label: 'Date taken'},
    {value: 'updated_at', label: 'Last modified'},
]

/** The gallery's search + filter controls, rendered inside the unified top bar. */
export function FilterControls() {
    const {params, update, hasActiveFilters, clearFilters} = useGalleryParams()

    const [q, setQ] = useState(params.q)
    const debouncedQ = useDebouncedValue(q, 300)

    useEffect(() => {
        if (debouncedQ !== params.q) update({q: debouncedQ}, {replace: true})
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [debouncedQ])

    useEffect(() => {
        setQ(params.q)
    }, [params.q])

    return (
        <>
          <div className="relative w-full min-w-0 max-w-[16rem]">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"/>
            <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search filenames…" className="h-8 pl-8"/>
            </div>

            {params.tag && (
                <Badge variant="secondary" className="hidden max-w-[12rem] gap-1 font-normal sm:inline-flex">
                  <span className="truncate">{TagPath.toDisplay(params.tag)}</span>
                  <button onClick={() => update({tag: null})} aria-label="Clear tag filter" className="ml-0.5 shrink-0">
                        <X className="h-3 w-3"/>
                    </button>
                </Badge>
            )}

          <div className="ml-auto flex shrink-0 items-center gap-1.5">
                <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                        <Button variant="outline" size="sm" className="gap-1.5">
                            <ArrowUpDown className="h-3.5 w-3.5"/>
                          <span className="hidden sm:inline">Sort</span>
                        </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-44">
                        <DropdownMenuLabel>Sort by</DropdownMenuLabel>
                        <DropdownMenuRadioGroup value={params.sort} onValueChange={(v) => update({sort: v as SortField})}>
                            {SORT_FIELDS.map((f) => (
                                <DropdownMenuRadioItem key={f.value} value={f.value}>
                                    {f.label}
                                </DropdownMenuRadioItem>
                            ))}
                        </DropdownMenuRadioGroup>
                        <DropdownMenuSeparator/>
                        <DropdownMenuRadioGroup value={params.order} onValueChange={(v) => update({order: v as SortOrder})}>
                            <DropdownMenuRadioItem value="desc">Newest first</DropdownMenuRadioItem>
                            <DropdownMenuRadioItem value="asc">Oldest first</DropdownMenuRadioItem>
                        </DropdownMenuRadioGroup>
                    </DropdownMenuContent>
                </DropdownMenu>

                <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                          variant="outline"
                          size="sm"
                          className={cn('gap-1.5', hasActiveFilters && 'border-primary/50 text-primary')}
                      >
                            <SlidersHorizontal className="h-3.5 w-3.5"/>
                        <span className="hidden sm:inline">Filters</span>
                        </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-52">
                      <DropdownMenuLabel>Show</DropdownMenuLabel>
                      <DropdownMenuRadioGroup value={params.scope} onValueChange={(v) => update({scope: v as Scope})}>
                        {SCOPES.map((s) => (
                            <DropdownMenuRadioItem key={s.value} value={s.value}>
                              {s.label}
                            </DropdownMenuRadioItem>
                        ))}
                      </DropdownMenuRadioGroup>
                      <DropdownMenuSeparator/>
                        <DropdownMenuCheckboxItem
                            checked={params.includeDeleted}
                            onCheckedChange={(checked) => update({includeDeleted: checked})}
                        >
                            Include trashed
                        </DropdownMenuCheckboxItem>
                        {/* TODO: capture-date range picker lives here */}
                        <DropdownMenuSeparator/>
                        <button
                            onClick={clearFilters}
                            disabled={!hasActiveFilters}
                            className="flex w-full items-center gap-2 px-2 py-1.5 text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
                        >
                            <Check className="h-3.5 w-3.5"/>
                            Clear all filters
                        </button>
                    </DropdownMenuContent>
                </DropdownMenu>
            </div>
        </>
    )
}
