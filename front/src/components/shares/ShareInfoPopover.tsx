import {useState} from 'react'
import {Info} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'

export interface ShareInfoEntry {
    /** Optional per-entry label (e.g. the recipient handle for a grouped row). */
    label?: string
    name: string
    message: string | null
}

/**
 * Name + message details for one or more shares, surfaced in a popover anchored
 * to the right (towards the pictures pane). Opens on hover (desktop) and on
 * click/tap (touch) via the explicit trigger button.
 */
export function ShareInfoPopover({entries}: { entries: ShareInfoEntry[] }) {
    const [open, setOpen] = useState(false)
    if (!entries.length) return null

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>
                <Button
                    size="icon"
                    variant="ghost"
                    className="h-6 w-6 text-muted-foreground hover:text-foreground"
                    title="Details"
                    onMouseEnter={() => setOpen(true)}
                >
                    <Info className="h-3.5 w-3.5"/>
                </Button>
            </PopoverTrigger>
            <PopoverContent
                side="right"
                align="start"
                className="w-64 space-y-2 p-3"
                onMouseLeave={() => setOpen(false)}
            >
                {entries.map((entry, i) => (
                    <div key={i} className="space-y-0.5">
                        {entry.label && (
                            <p className="truncate text-[11px] text-muted-foreground">{entry.label}</p>
                        )}
                        <p className="break-words text-sm font-medium">{entry.name}</p>
                        {entry.message ? (
                            <p className="whitespace-pre-wrap break-words text-xs text-muted-foreground">
                                {entry.message}
                            </p>
                        ) : (
                            <p className="text-xs italic text-muted-foreground/60">No message</p>
                        )}
                    </div>
                ))}
            </PopoverContent>
        </Popover>
    )
}

/** The most common `name` among a set of shares, with a "(and N others)" suffix
 *  when the group mixes several distinct names. */
export function summarizeNames(names: string[]): string {
    if (!names.length) return ''
    const counts = new Map<string, number>()
    for (const n of names) counts.set(n, (counts.get(n) ?? 0) + 1)
    const [top] = [...counts.entries()].sort((a, b) => b[1] - a[1])
    const others = names.length - top[1]
    return others > 0 ? `${top[0]} (and ${others} other${others !== 1 ? 's' : ''})` : top[0]
}
