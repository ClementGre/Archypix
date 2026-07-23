// Sort dropdown for the grid header (field + direction). Proximity sorts (feature 29) are set from a
// picture's "Find nearby" action rather than chosen here, so the menu only surfaces the active one
// with a one-click clear. Writes the `sort`/`order` URL params.

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
import type {SortField, SortOrder} from '@/lib/types'
import {cn} from '@/lib/utils'

const SORT_FIELDS: { value: SortField; label: string }[] = [
    {value: 'captured_at', label: 'Date taken'},
    {value: 'ingested_at', label: 'Date added'},
    {value: 'updated_at', label: 'Last modified'},
    {value: 'file_size', label: 'File size'},
    {value: 'filename', label: 'Name'},
]

export function SortMenu() {
    const {params, update} = useGalleryParams()
    const isProximity = params.sort === 'time_near' || params.sort === 'geo_near'

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button
                    variant="outline"
                    size="sm"
                    className={cn(
                        'gap-1.5 text-xs font-normal',
                        isProximity ? 'border-primary/50 text-primary' : 'text-muted-foreground',
                    )}
                >
                    <ArrowUpDown className="h-3.5 w-3.5"/>
                    <span className="hidden sm:inline">Sort</span>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
                {isProximity && (
                    <>
                        <DropdownMenuLabel>Proximity</DropdownMenuLabel>
                        <DropdownMenuItem
                            className="gap-2 text-primary"
                            onSelect={() =>
                                update({sort: 'captured_at', nearTime: null, nearLat: null, nearLng: null})
                            }
                        >
                            <span className="flex-1">
                                {params.sort === 'time_near' ? 'Nearby in time' : 'Nearby in place'}
                            </span>
                            <X className="h-3.5 w-3.5"/>
                        </DropdownMenuItem>
                        <DropdownMenuSeparator/>
                    </>
                )}
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
    )
}
