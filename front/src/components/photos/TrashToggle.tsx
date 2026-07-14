// Three-state trash filter, rendered at the right of the grid header. The trash is a filter over the
// main view (feature "Better trash") rather than a separate page: hide trashed (default), show
// everything, or show the trash only. Writes the `trash` URL param via useGalleryParams.

import {Images, Layers, Trash2} from 'lucide-react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import type {TrashFilter} from '@/lib/types'
import {cn} from '@/lib/utils'

const OPTIONS: { value: TrashFilter; label: string; tip: string; Icon: typeof Images }[] = [
    {value: 'exclude', label: 'Photos', tip: 'Hide trashed', Icon: Images},
    {value: 'include', label: 'All', tip: 'Include trashed', Icon: Layers},
    {value: 'only', label: 'Trash', tip: 'Trashed only', Icon: Trash2},
]

export function TrashToggle() {
    const {params, update} = useGalleryParams()

    return (
        <div
            role="group"
            aria-label="Trash filter"
            className="flex shrink-0 items-center gap-0.5 rounded-md border border-border p-0.5"
        >
            {OPTIONS.map(({value, label, tip, Icon}) => {
                const active = params.trash === value
                const activeCls = value === 'only' ? 'bg-destructive/15 text-destructive' : 'bg-primary/15 text-primary'
                return (
                    <Tooltip key={value} delayDuration={300}>
                        <TooltipTrigger asChild>
                            <button
                                type="button"
                                onClick={() => update({trash: value})}
                                aria-pressed={active}
                                className={cn(
                                    'flex items-center gap-1 rounded px-2 py-1 text-xs transition-colors',
                                    active ? activeCls : 'text-muted-foreground hover:text-foreground',
                                )}
                            >
                                <Icon className="h-3.5 w-3.5"/>
                                <span className="hidden sm:inline">{label}</span>
                            </button>
                        </TooltipTrigger>
                        <TooltipContent className="text-xs">{tip}</TooltipContent>
                    </Tooltip>
                )
            })}
        </div>
    )
}
