// The **View** dropdown (feature 35 §4): the recursive view mode of the tag the user is standing on.
// It replaces the tag tree's `(=)` exact toggle and the `TagFilterBar` include↔exact switch — a less
// fiddly home than a per-node icon.
//
// The mode is remembered per tag in `tag_metadata`, so the trigger simply names the active one. It
// is not highlighted as "non-default": a stored preference is the normal state, not an anomaly.

import {Layers, Rows3} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuLabel,
    DropdownMenuRadioGroup,
    DropdownMenuRadioItem,
    DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useTimelineView} from '@/hooks/useTimelineView'
import {cn} from '@/lib/utils'
import type {TagViewMode} from '@/lib/types'

const MODES: { value: TagViewMode; label: string; hint: (root: boolean) => string }[] = [
    {
        value: 'direct',
        label: 'Direct only',
        hint: (root) => (root ? 'Photos with no tag at all' : 'Only photos carrying this tag itself'),
    },
    {value: 'subtag', label: 'Subtags', hint: () => 'Direct photos plus a block per subtag'},
    {value: 'all', label: 'Everything', hint: (root) => (root ? 'Every photo' : 'Every photo under this tag, flat')},
]

export function ViewMenu() {
    const {params} = useGalleryParams()
    const {mode, pinned, setMode} = useTimelineView()
    // Hierarchies have their own directory structure and the two must not compete (§10.9).
    if (params.hierarchy) return null

    const isRoot = !params.tag
    const label = MODES.find((m) => m.value === mode)?.label ?? 'View'

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm" className="gap-1.5 text-xs font-normal text-muted-foreground">
                    {mode === 'all' ? <Rows3 className="h-3.5 w-3.5"/> : <Layers className="h-3.5 w-3.5"/>}
                    <span className="hidden sm:inline">{label}</span>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-60">
                <DropdownMenuLabel>Recursive view mode</DropdownMenuLabel>
                <DropdownMenuRadioGroup value={mode} onValueChange={(v) => setMode(v as TagViewMode)}>
                    {MODES.map((m) => {
                        // Fix mode rules out `subtag` only — it needs a flat stream to find temporal
                        // and spatial neighbours, which `direct` still gives it (§9).
                        const blocked = pinned && m.value === 'subtag'
                        return (
                            <DropdownMenuRadioItem
                                key={m.value}
                                value={m.value}
                                disabled={blocked}
                                className="flex-col items-start gap-0"
                            >
                                <span className={cn(blocked && 'text-muted-foreground')}>{m.label}</span>
                                <span className="text-[11px] text-muted-foreground">
                                    {blocked ? 'Unavailable while a fix mode is on' : m.hint(isRoot)}
                                </span>
                            </DropdownMenuRadioItem>
                        )
                    })}
                </DropdownMenuRadioGroup>
            </DropdownMenuContent>
        </DropdownMenu>
    )
}
