import type {ReactNode} from 'react'
import {ChevronDown} from 'lucide-react'
import {cn} from '@/lib/utils'
import {usePersistentBool} from '@/hooks/usePersistentBool'

/**
 * Compact foldable section. Uncontrolled by default (open/closed persisted per `id`); pass
 * `open` + `onOpenChange` to drive it externally (e.g. to lazily fetch when expanded).
 */
export function Section({
                            id,
                            title,
                            count,
                            defaultOpen = true,
                            action,
                            children,
                            open: openProp,
                            onOpenChange,
                        }: {
    id: string
    title: string
    count?: number
    defaultOpen?: boolean
    action?: ReactNode
    children: ReactNode
    open?: boolean
    onOpenChange?: (open: boolean) => void
}) {
    const [persisted, setPersisted] = usePersistentBool(`section_${id}`, defaultOpen)
    const controlled = openProp !== undefined
    const open = controlled ? openProp : persisted
    const toggle = () => {
        if (controlled) onOpenChange?.(!open)
        else setPersisted()
    }

    return (
        <div className="border-b border-border">
            <div className="flex items-center gap-1">
                <button onClick={() => toggle()} className="flex flex-1 items-center gap-1.5 py-2 text-sm font-medium">
                    <ChevronDown className={cn('h-4 w-4 text-muted-foreground transition-transform', !open && '-rotate-90')}/>
                    {title}
                    {count !== undefined && <span className="text-xs font-normal text-muted-foreground">{count}</span>}
                </button>
                {action && <div className="flex items-center">{action}</div>}
            </div>
            {open && <div className="pb-3">{children}</div>}
        </div>
    )
}
