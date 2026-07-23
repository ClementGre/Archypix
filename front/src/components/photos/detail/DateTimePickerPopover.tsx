import type {ReactNode} from 'react'
import {useState} from 'react'
import {Calendar} from '@/components/ui/calendar'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Button} from '@/components/ui/button'
import {buildNaive, parseNaive} from '@/lib/fixDate'

/** Format "YYYY-MM-DDTHH:MM:SS" for display; empty string when unset (callers add any placeholder). */
export function formatNaive(iso: string | null | undefined): string {
    if (!iso) return ''
    const {date, time} = parseNaive(iso)
    const dateStr = date.toLocaleDateString(undefined, {year: 'numeric', month: 'short', day: 'numeric'})
    return `${dateStr} ${time}`
}

interface DateTimePickerPopoverProps {
    /** Current value as "YYYY-MM-DDTHH:MM:SS" or null. */
    value: string | null
    onChange: (value: string | null) => void
    children: ReactNode
    /** Disable days before today (e.g. an expiry can't be in the past). */
    disablePast?: boolean
    /**
     * Optional "From …" prefill chips (feature 30 §6): a labelled candidate date the user can apply
     * with one click (e.g. from the filename / source file date / upload date). Shown above the
     * calendar, typically only when the field is empty.
     */
    suggestions?: { label: string; value: string; lowConfidence?: boolean }[]
}

export function DateTimePickerPopover({value, onChange, children, disablePast, suggestions}: DateTimePickerPopoverProps) {
    const [open, setOpen] = useState(false)

    const parsed = value ? parseNaive(value) : null
    const selectedDate = parsed?.date
    const time = parsed?.time ?? '12:00'
    const disabledDays = disablePast ? {before: new Date(new Date().setHours(0, 0, 0, 0))} : undefined

    function handleSelectDate(date: Date | undefined) {
        if (!date) {
            onChange(null)
            return
        }
        onChange(buildNaive(date, time))
    }

    function handleTimeChange(newTime: string) {
        const base = selectedDate ?? new Date()
        onChange(buildNaive(base, newTime))
    }

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>{children}</PopoverTrigger>
            <PopoverContent className="max-h-[85vh] w-auto max-w-[min(92vw,18rem)] space-y-3 overflow-y-auto p-3" side="left" align="start"
                            collisionPadding={8}>
                {suggestions && suggestions.length > 0 && (
                    <div className="flex flex-wrap gap-1 border-b border-border pb-2">
                        {suggestions.map((s) => (
                            <button
                                key={s.label}
                                type="button"
                                onClick={() => onChange(s.value)}
                                title={formatNaive(s.value)}
                                className="max-w-full rounded-full border border-border px-2 py-0.5 text-[11px] text-muted-foreground transition-colors hover:border-primary/60 hover:text-primary"
                            >
                                <span className={s.lowConfidence ? 'text-amber-500' : undefined}>{s.label}</span>: {formatNaive(s.value)}
                            </button>
                        ))}
                    </div>
                )}
                <Calendar
                    mode="single"
                    weekStartsOn={1}
                    selected={selectedDate}
                    onSelect={handleSelectDate}
                    disabled={disabledDays}
                    captionLayout="dropdown"
                    className="[--cell-size:1.5rem]"
                    classNames={{
                        weekdays: 'flex gap-1',
                        week: 'mt-1.5 flex w-full gap-1',
                    }}
                />
                <div className="flex items-end gap-2">
                    <div className="flex-1 space-y-1">
                        <Label className="text-xs text-muted-foreground">Time</Label>
                        <Input
                            type="time"
                            value={time}
                            onChange={(e) => handleTimeChange(e.target.value)}
                            className="h-9"
                        />
                    </div>
                    <Button
                        variant="outline"
                        size="sm"
                        className="h-9"
                        onClick={() => {
                            onChange(null)
                            setOpen(false)
                        }}
                    >
                        Clear
                    </Button>
                </div>
            </PopoverContent>
        </Popover>
    )
}
