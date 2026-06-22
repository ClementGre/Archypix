import type {ReactNode} from 'react'
import {useState} from 'react'
import {Calendar} from '@/components/ui/calendar'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Button} from '@/components/ui/button'

/** Parse "YYYY-MM-DDTHH:MM:SS" (NaiveDateTime, no tz) into { date, time }. */
function parseNaive(iso: string): { date: Date; time: string } {
    const [datePart = '', timePart = '00:00:00'] = iso.split('T')
    const [y = 2000, mo = 1, d = 1] = datePart.split('-').map(Number)
    return {
        date: new Date(y, mo - 1, d),
        time: timePart.slice(0, 5), // "HH:MM"
    }
}

/** Build "YYYY-MM-DDTHH:MM:SS" from a local Date + "HH:MM" string. */
function buildNaive(date: Date, time: string): string {
    const y = date.getFullYear()
    const mo = String(date.getMonth() + 1).padStart(2, '0')
    const d = String(date.getDate()).padStart(2, '0')
    const t = time.length >= 5 ? time.slice(0, 5) + ':00' : '00:00:00'
    return `${y}-${mo}-${d}T${t}`
}

/** Format "YYYY-MM-DDTHH:MM:SS" for display. */
export function formatNaive(iso: string | null | undefined): string {
    if (!iso) return '—'
    const {date, time} = parseNaive(iso)
    const dateStr = date.toLocaleDateString(undefined, {year: 'numeric', month: 'short', day: 'numeric'})
    return `${dateStr} ${time}`
}

interface DateTimePickerPopoverProps {
    /** Current value as "YYYY-MM-DDTHH:MM:SS" or null. */
    value: string | null
    onChange: (value: string | null) => void
    children: ReactNode
}

export function DateTimePickerPopover({value, onChange, children}: DateTimePickerPopoverProps) {
    const [open, setOpen] = useState(false)

    const parsed = value ? parseNaive(value) : null
    const selectedDate = parsed?.date
    const time = parsed?.time ?? '12:00'

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
            <PopoverContent className="w-auto space-y-3 p-3" side="left" align="start">
                <Calendar
                    mode="single"
                    weekStartsOn={1}
                    selected={selectedDate}
                    onSelect={handleSelectDate}
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
