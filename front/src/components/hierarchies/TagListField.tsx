import {X} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {TagPicker} from '@/components/tags/TagPicker'
import {cn, TagPath} from '@/lib/utils'

/** A labelled set of tag chips with an add-picker; values are wire-form paths. */
export function TagListField({
                                 label,
                                 values,
                                 onChange,
                                 color = 'muted',
                                 allowProtected = true,
                                 allowCreate = true,
                                 placeholder,
                                 emptyHint,
                             }: {
    label: string
    values: string[]
    onChange: (next: string[]) => void
    color?: 'muted' | 'emerald' | 'red'
    allowProtected?: boolean
    allowCreate?: boolean
    placeholder?: string
    emptyHint?: string
}) {
    const chipClass = {
        muted: 'bg-muted text-foreground',
        emerald: 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400',
        red: 'bg-red-500/10 text-red-600 dark:text-red-400',
    }[color]
    const hoverClass = {
        muted: 'hover:bg-foreground/10',
        emerald: 'hover:bg-emerald-500/20',
        red: 'hover:bg-red-500/20',
    }[color]

    return (
        <div className="flex flex-wrap items-center gap-1.5">
            <span className="w-20 shrink-0 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {label}
            </span>
            {values.length === 0 && emptyHint && (
                <span className="text-xs italic text-muted-foreground">{emptyHint}</span>
            )}
            {values.map((wire) => (
                <Badge key={wire} variant="secondary" className={cn('gap-1 pr-1', chipClass)}>
                    {TagPath.toDisplay(wire)}
                    <button
                        onClick={() => onChange(values.filter((v) => v !== wire))}
                        className={cn('ml-0.5 rounded-full p-0.5', hoverClass)}
                        aria-label="Remove"
                    >
                        <X className="h-2.5 w-2.5"/>
                    </button>
                </Badge>
            ))}
            <TagPicker
                onSelect={(wire) => !values.includes(wire) && onChange([...values, wire])}
                excludePaths={values}
                allowCreate={allowCreate}
                allowProtected={allowProtected}
                triggerLabel="Add"
                placeholder={placeholder}
            />
        </div>
    )
}
