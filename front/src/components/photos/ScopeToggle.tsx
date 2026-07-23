// Ownership scope filter (All / Mine / Shared with me) as a grid-header dropdown, same style as the
// Trash control. Writes the `scope` URL param.

import {Check, Inbox, User, Users} from 'lucide-react'
import {type Scope, useGalleryParams} from '@/hooks/useGalleryParams'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import {cn} from '@/lib/utils'

const OPTIONS: { value: Scope; label: string; Icon: typeof Users }[] = [
    {value: 'all', label: 'All', Icon: Users},
    {value: 'owned', label: 'Mine', Icon: User},
    // "Received" (not "Shared") — these are pictures shared *to* the user; matches the app's
    // "received pictures" terminology and avoids implying the user shared them out.
    {value: 'shared', label: 'Received', Icon: Inbox},
]

export function ScopeToggle() {
    const {params, update} = useGalleryParams()
    const active = OPTIONS.find((o) => o.value === params.scope) ?? OPTIONS[0]
    const isDefault = params.scope === 'all'

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button
                    variant="outline"
                    size="sm"
                    aria-label="Ownership filter"
                    className={cn(
                        'gap-1.5 text-xs font-normal',
                        isDefault ? 'text-muted-foreground' : 'border-primary/50 text-primary',
                    )}
                >
                    <active.Icon className="h-3.5 w-3.5"/>
                    {/* At the default (All), show the filter's name rather than the value. */}
                    <span className="hidden sm:inline">{isDefault ? 'Ownership' : active.label}</span>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-40">
                <DropdownMenuLabel>Ownership</DropdownMenuLabel>
                {OPTIONS.map(({value, label, Icon}) => (
                    <DropdownMenuItem key={value} onSelect={() => update({scope: value})} className="gap-2">
                        <Icon className="h-4 w-4"/>
                        <span className="flex-1">{label}</span>
                        {params.scope === value && <Check className="h-4 w-4"/>}
                    </DropdownMenuItem>
                ))}
            </DropdownMenuContent>
        </DropdownMenu>
    )
}
