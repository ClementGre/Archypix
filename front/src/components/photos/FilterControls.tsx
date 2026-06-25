import {ArrowUpDown, Check, Filter} from 'lucide-react'
import {Button} from '@/components/ui/button'
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
import {DateRangePicker} from '@/components/common/DateRangePicker'
import {type Scope, useGalleryParams} from '@/hooks/useGalleryParams'
import type {SortField, SortOrder} from '@/lib/types'
import {cn} from '@/lib/utils'

const SCOPES: { value: Scope; label: string }[] = [
  {value: 'all', label: 'All photos'},
    {value: 'owned', label: 'Mine'},
  {value: 'shared', label: 'Shared with me'},
]

const SORT_FIELDS: { value: SortField; label: string }[] = [
    {value: 'captured_at', label: 'Date taken'},
    {value: 'ingested_at', label: 'Date added'},
    {value: 'updated_at', label: 'Last modified'},
    {value: 'file_size', label: 'File size'},
    {value: 'filename', label: 'Name'},
]

/** The gallery's sort + filter controls, rendered inside the unified top bar. */
export function FilterControls() {
    const {params, update, hasActiveFilters, clearFilters} = useGalleryParams()

    // The capture-date bounds are stored as RFC3339 (UTC) on the wire; the picker works in plain
    // `YYYY-MM-DD`, so map between the two (start of day / end of day).
    const fromDate = params.capturedAfter?.slice(0, 10) ?? ''
    const toDate = params.capturedBefore?.slice(0, 10) ?? ''
    const onDateRange = (from: string, to: string) =>
        update({
            capturedAfter: from ? `${from}T00:00:00Z` : null,
            capturedBefore: to ? `${to}T23:59:59Z` : null,
        })

    return (
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
                        <DropdownMenuRadioItem value="desc">Descending</DropdownMenuRadioItem>
                        <DropdownMenuRadioItem value="asc">Ascending</DropdownMenuRadioItem>
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
                        <Filter className="h-3.5 w-3.5"/>
                        <span className="hidden sm:inline">Filters</span>
                    </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-60">
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
                    <DropdownMenuSeparator/>
                    {/* Capture-date range — same calendar widget as the query-rule date ranges. */}
                    <div className="px-2 py-1.5">
                        <p className="mb-1.5 text-xs font-medium text-muted-foreground">Capture date</p>
                        <DateRangePicker mode="date" from={fromDate} to={toDate} onChange={onDateRange}/>
                    </div>
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
    )
}
