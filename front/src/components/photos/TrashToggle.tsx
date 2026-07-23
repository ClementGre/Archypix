// Three-state trash filter as a grid-header dropdown (feature "Better trash" — the trash is a filter
// over the main view, not a separate page): hide trashed (default), show everything, or show the
// trash only. Writes the `trash` URL param via useGalleryParams.

import {Check, Images, Layers, Trash2} from 'lucide-react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import type {TrashFilter} from '@/lib/types'
import {cn} from '@/lib/utils'

const OPTIONS: { value: TrashFilter; label: string; Icon: typeof Images }[] = [
    {value: 'exclude', label: 'Photos', Icon: Images},
    {value: 'include', label: 'All', Icon: Layers},
    {value: 'only', label: 'Trashed', Icon: Trash2},
]

export function TrashToggle() {
    const {params, update} = useGalleryParams()
    const active = OPTIONS.find((o) => o.value === params.trash) ?? OPTIONS[0]
    const isDefault = params.trash === 'exclude'
    // The trigger carries the control's identity (a trash can) at its default; once filtering it
    // shows the active option's icon.
    const TriggerIcon = isDefault ? Trash2 : active.Icon

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button
                    variant="outline"
                    size="sm"
                    aria-label="Trash filter"
                    className={cn(
                        'gap-1.5 text-xs font-normal',
                        isDefault
                            ? 'text-muted-foreground'
                            : params.trash === 'only'
                                ? 'border-destructive/50 text-destructive'
                                : 'border-primary/50 text-primary',
                    )}
                >
                    <TriggerIcon className="h-3.5 w-3.5"/>
                    {/* At the default (hide trashed), show the filter's name rather than "Photos". */}
                    <span className="hidden sm:inline">{isDefault ? 'Trash' : active.label}</span>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-40">
                <DropdownMenuLabel>Trash</DropdownMenuLabel>
                {OPTIONS.map(({value, label, Icon}) => (
                    <DropdownMenuItem key={value} onSelect={() => update({trash: value})} className="gap-2">
                        <Icon className="h-4 w-4"/>
                        <span className="flex-1">{label}</span>
                        {params.trash === value && <Check className="h-4 w-4"/>}
                    </DropdownMenuItem>
                ))}
            </DropdownMenuContent>
        </DropdownMenu>
    )
}
