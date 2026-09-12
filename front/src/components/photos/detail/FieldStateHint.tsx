import {type ReactNode, useRef, useState} from 'react'
import {Popover, PopoverAnchor, PopoverContent} from '@/components/ui/popover'
import {cn} from '@/lib/utils'

/** Which annotation a row carries; drives the tint and the popup's wording. */
export type FieldTone = 'override' | 'diff'

export interface FieldState {
    tone: FieldTone
    /** The value the revert action would restore (the owner's, or the file's). */
    reference: string
    onRevert: () => void
}

const TONE = {
    override: {
        tint: 'bg-amber-500/15 hover:bg-amber-500/25',
        title: 'Overwritten locally',
        body: "You overrode this field locally. The change is private to you and is never written to the owner's file, so it is not visible in WebDAV (which serves the owner's picture directly, without applying your overrides).",
        referenceLabel: "Owner's value",
        action: "Use the owner's value",
    },
    diff: {
        tint: 'bg-red-500/15 hover:bg-red-500/25',
        title: 'Not written to the file',
        body: 'Writing this picture\'s metadata to its file failed permanently, so the stored value never reached the file. Retry the sync, or revert the field to what the file actually holds.',
        referenceLabel: 'Value in the file',
        action: 'Revert to the file value',
    },
} as const

// Grace period so the pointer can travel from the value to the popup without it closing.
const CLOSE_DELAY_MS = 120

/**
 * Tints an EXIF row's value and explains the tint in a hover popup carrying the matching revert
 * action. A tint costs no horizontal space, unlike the inline badge this replaces — the rows are
 * narrow and a badge pushed the value out of the panel.
 *
 * Both reverts are **draft-only**: they change the field in place and are persisted by Save, like
 * any other edit in the panel.
 */
export function FieldStateHint({state, children}: { state?: FieldState; children: ReactNode }) {
    const [open, setOpen] = useState(false)
    const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

    if (!state) return <>{children}</>
    const tone = TONE[state.tone]

    const show = () => {
        if (timer.current) clearTimeout(timer.current)
        setOpen(true)
    }
    const hide = () => {
        if (timer.current) clearTimeout(timer.current)
        timer.current = setTimeout(() => setOpen(false), CLOSE_DELAY_MS)
    }

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverAnchor asChild>
                <span
                    onMouseEnter={show}
                    onMouseLeave={hide}
                    // Clicking the value starts editing (or opens the date/GPS picker) — get out of
                    // its way rather than overlapping it.
                    onClick={() => setOpen(false)}
                    className={cn('inline-flex min-w-0 rounded transition-colors', tone.tint)}
                >
                    {children}
                </span>
            </PopoverAnchor>
            <PopoverContent
                side="left"
                align="center"
                onMouseEnter={show}
                onMouseLeave={hide}
                onOpenAutoFocus={(e) => e.preventDefault()}
                className="w-64 space-y-2 p-3 text-xs"
            >
                <div className="font-medium">{tone.title}</div>
                <p className="text-muted-foreground">{tone.body}</p>
                <div className="flex items-baseline justify-between gap-2 border-t pt-2">
                    <span className="text-muted-foreground">{tone.referenceLabel}</span>
                    <span className="min-w-0 truncate font-medium">{state.reference || '—'}</span>
                </div>
                <button
                    onClick={() => {
                        setOpen(false)
                        state.onRevert()
                    }}
                    className="w-full rounded border px-2 py-1 text-xs transition-colors hover:bg-muted"
                >
                    {tone.action}
                </button>
                <p className="text-[10px] text-muted-foreground">Applied when you save.</p>
            </PopoverContent>
        </Popover>
    )
}
