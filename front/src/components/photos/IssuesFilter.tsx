// Metadata-completeness filter (feature 29 §4/§8), a grid-header dropdown next to the TrashToggle.
// Each of GPS and capture-date is an independent three-state filter — Any / Present / Missing — so
// the user can both hunt for problem pictures (Missing) *and* isolate the good anchors (Present, the
// invert). Writes the `gps` / `cdate` URL params via useGalleryParams.

import {CalendarClock, MapPin, SlidersHorizontal} from 'lucide-react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import type {PresenceFilter} from '@/lib/types'
import {cn} from '@/lib/utils'

const STATES: { value: PresenceFilter; label: string }[] = [
    {value: 'any', label: 'Any'},
    {value: 'present', label: 'Present'},
    {value: 'missing', label: 'Missing'},
]

/** One labelled three-state row (Any / Present / Missing). */
function PresenceRow({
                         Icon,
                         label,
                         value,
                         onChange,
                     }: {
    Icon: typeof MapPin
    label: string
    value: PresenceFilter
    onChange: (v: PresenceFilter) => void
}) {
    return (
        <div className="flex items-center gap-2 px-2 py-1.5">
            <Icon className="h-4 w-4 shrink-0 text-muted-foreground"/>
            <span className="flex-1 text-sm">{label}</span>
            <div className="flex items-center gap-0.5 rounded-md border border-border p-0.5">
                {STATES.map(({value: v, label: l}) => (
                    <button
                        key={v}
                        type="button"
                        onClick={() => onChange(v)}
                        aria-pressed={value === v}
                        className={cn(
                            'rounded px-1.5 py-0.5 text-xs transition-colors',
                            value === v
                                ? v === 'missing'
                                    ? 'bg-destructive/15 text-destructive'
                                    : 'bg-primary/15 text-primary'
                                : 'text-muted-foreground hover:text-foreground',
                        )}
                    >
                        {l}
                    </button>
                ))}
            </div>
        </div>
    )
}

export function IssuesFilter() {
    const {params, update} = useGalleryParams()

    // Setting a per-field state clears the `missing_any` OR (they are mutually exclusive server-side).
    const activeCount = (params.gps !== 'any' ? 1 : 0) + (params.captureDate !== 'any' ? 1 : 0)
    const filtering = activeCount > 0 || params.missingAny

    const summary = params.missingAny
        ? 'Any issue'
        : activeCount === 0
            ? 'Metadata'
            : activeCount === 1
                ? params.gps !== 'any'
                    ? `GPS ${params.gps}`
                    : `Date ${params.captureDate}`
                : `${activeCount} filters`

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button
                    variant="outline"
                    size="sm"
                    aria-label="Metadata filter"
                    className={cn(
                        'gap-1.5 text-xs font-normal capitalize',
                        filtering ? 'border-primary/50 text-primary' : 'text-muted-foreground',
                    )}
                >
                    <SlidersHorizontal className="h-3.5 w-3.5"/>
                    <span className="hidden sm:inline">{summary}</span>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-64">
                <DropdownMenuLabel>Metadata</DropdownMenuLabel>
                <PresenceRow
                    Icon={MapPin}
                    label="GPS"
                    value={params.gps}
                    onChange={(v) => update({gps: v, missingAny: false})}
                />
                <PresenceRow
                    Icon={CalendarClock}
                    label="Capture date"
                    value={params.captureDate}
                    onChange={(v) => update({captureDate: v, missingAny: false})}
                />
                <DropdownMenuSeparator/>
                <button
                    type="button"
                    onClick={() => update({gps: 'any', captureDate: 'any', missingAny: !params.missingAny})}
                    className={cn(
                        'flex w-full items-center gap-2 px-2 py-1.5 text-sm transition-colors hover:bg-accent',
                        params.missingAny && 'text-primary',
                    )}
                >
                    <span className="flex-1 text-left">Any issue (missing either)</span>
                    <span
                        className={cn(
                            'h-3.5 w-3.5 rounded-sm border',
                            params.missingAny ? 'border-primary bg-primary' : 'border-border',
                        )}
                    />
                </button>
            </DropdownMenuContent>
        </DropdownMenu>
    )
}
