// A calendar-based date(-time) range picker built on the shadcn `Calendar` (react-day-picker),
// week starting Monday. Two modes:
//   • 'date'     — emits inclusive `YYYY-MM-DD` bounds.
//   • 'datetime' — emits NaiveDateTime `YYYY-MM-DDTHH:MM:SS` bounds. Times are optional: when "Set
//                  times" is off, the first day starts at 00:00:00 and the last ends at 23:59:59.
//                  No timezone is ever applied (the backend expects a NaiveDateTime).
//
// Selection is deterministic: a "Click sets: Start | End" switch decides which end the next click
// writes (it auto-advances Start → End), so the first day stays editable — click "Start" to re-pick
// it. A Clear button resets the whole range. (react-day-picker's own range click-logic is bypassed
// via `onDayClick`.)

import {useState} from 'react'
import {CalendarRange} from 'lucide-react'
import type {DateRange} from 'react-day-picker'
import {Calendar} from '@/components/ui/calendar'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'

interface DateRangePickerProps {
    mode: 'date' | 'datetime'
    /** 'YYYY-MM-DD' (date) or 'YYYY-MM-DDTHH:MM:SS' (datetime); '' when unset. */
    from: string
    to: string
    onChange: (from: string, to: string) => void
    placeholder?: string
}

const pad = (n: number) => String(n).padStart(2, '0')

function parseDate(s: string): Date | undefined {
    if (!s) return undefined
    const [datePart = ''] = s.split('T')
    const [y, mo, d] = datePart.split('-').map(Number)
    if (!y || !mo || !d) return undefined
    return new Date(y, mo - 1, d)
}

function parseTime(s: string): string {
    const t = s.split('T')[1]
    return t ? t.slice(0, 5) : ''
}

const dateStr = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`

function fmtDisplay(s: string, mode: 'date' | 'datetime'): string {
    const d = parseDate(s)
    if (!d) return ''
    const base = d.toLocaleDateString(undefined, {year: 'numeric', month: 'short', day: 'numeric'})
    if (mode === 'datetime') {
        const t = parseTime(s)
        if (t && t !== '00:00' && t !== '23:59') return `${base} ${t}`
    }
    return base
}

export function DateRangePicker({mode, from, to, onChange, placeholder = 'Pick a range'}: DateRangePickerProps) {
    const [open, setOpen] = useState(false)
    const [picking, setPicking] = useState<'start' | 'end'>('start')
    const [withTimes, setWithTimes] = useState(
        () => mode === 'datetime' && (isCustomTime(parseTime(from), false) || isCustomTime(parseTime(to), true)),
    )

    const startDate = parseDate(from)
    const endDate = parseDate(to)
    // Highlight the end even when only it is set (open-ended start).
    const selected: DateRange | undefined = startDate
        ? {from: startDate, to: endDate}
        : endDate
            ? {from: endDate}
            : undefined
    const fromTime = parseTime(from) || '00:00'
    const toTime = parseTime(to) || '23:59'

    // Order the two bounds (so we never emit from > to) and format per mode.
    const emit = (s: Date | undefined, e: Date | undefined, ft: string, tt: string, times: boolean) => {
        let a = s
        let b = e
        if (a && b && a.getTime() > b.getTime()) [a, b] = [b, a]
        if (mode === 'date') {
            onChange(a ? dateStr(a) : '', b ? dateStr(b) : '')
            return
        }
        onChange(
            a ? `${dateStr(a)}T${times ? `${ft}:00` : '00:00:00'}` : '',
            b ? `${dateStr(b)}T${times ? `${tt}:00` : '23:59:59'}` : '',
        )
    }

    const handleDayClick = (day: Date) => {
        if (picking === 'start') {
            emit(day, endDate, fromTime, toTime, withTimes)
            setPicking('end')
        } else {
            emit(startDate, day, fromTime, toTime, withTimes)
        }
    }

    const clear = () => {
        onChange('', '')
        setPicking('start')
    }

    const label =
        from && to
            ? `${fmtDisplay(from, mode)} → ${fmtDisplay(to, mode)}`
            : from
                ? fmtDisplay(from, mode)
                : to
                    ? `→ ${fmtDisplay(to, mode)}`
                    : placeholder

    return (
        <Popover
            open={open}
            onOpenChange={(o) => {
                setOpen(o)
                if (o) setPicking(startDate && !endDate ? 'end' : 'start')
            }}
        >
            <PopoverTrigger asChild>
                <Button variant="outline" size="sm" className="h-8 justify-start gap-1.5 text-xs font-normal">
                    <CalendarRange className="h-3.5 w-3.5 text-muted-foreground"/>
                    <span className={from || to ? '' : 'text-muted-foreground'}>{label}</span>
                </Button>
            </PopoverTrigger>
            <PopoverContent className="w-auto space-y-2 p-3" align="start">
                {/* Which end the next click sets — keeps the first day editable. */}
                <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-1.5">
                        <span className="text-xs text-muted-foreground">Click sets:</span>
                        <div className="inline-flex overflow-hidden rounded-md border text-xs">
                            {(['start', 'end'] as const).map((p) => (
                                <button
                                    key={p}
                                    type="button"
                                    onClick={() => setPicking(p)}
                                    className={`px-2 py-0.5 font-medium capitalize transition-colors ${
                                        picking === p ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-accent'
                                    }`}
                                >
                                    {p}
                                </button>
                            ))}
                        </div>
                    </div>
                    <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={clear} disabled={!from && !to}>
                        Clear
                    </Button>
                </div>

                <Calendar
                    mode="range"
                    weekStartsOn={1}
                    selected={selected}
                    // Selection is driven by `onDayClick` + the Start/End toggle; this no-op keeps
                    // the calendar in *controlled* range mode so its highlight tracks `selected`
                    // (without it, react-day-picker keeps its own divergent internal range).
                    onSelect={() => undefined}
                    onDayClick={handleDayClick}
                    captionLayout="dropdown"
                    className="[--cell-size:1.6rem]"
                    classNames={{weekdays: 'flex gap-1', week: 'mt-1.5 flex w-full gap-1'}}
                />

                {mode === 'datetime' && (
                    <div className="space-y-2 border-t pt-2">
                        <label className="flex items-center gap-2 text-xs text-muted-foreground">
                            <Switch
                                checked={withTimes}
                                onCheckedChange={(v) => {
                                    setWithTimes(v)
                                    emit(startDate, endDate, fromTime, toTime, v)
                                }}
                            />
                            Set times
                        </label>
                        {withTimes && (
                            <div className="grid grid-cols-2 gap-2">
                                <div className="space-y-1">
                                    <Label className="text-xs text-muted-foreground">Start time</Label>
                                    <Input
                                        type="time"
                                        value={fromTime}
                                        onChange={(e) => emit(startDate, endDate, e.target.value, toTime, true)}
                                        className="h-8 text-xs"
                                    />
                                </div>
                                <div className="space-y-1">
                                    <Label className="text-xs text-muted-foreground">End time</Label>
                                    <Input
                                        type="time"
                                        value={toTime}
                                        onChange={(e) => emit(startDate, endDate, fromTime, e.target.value, true)}
                                        className="h-8 text-xs"
                                    />
                                </div>
                            </div>
                        )}
                    </div>
                )}
            </PopoverContent>
        </Popover>
    )
}

/** A time string that isn't one of the implicit day-bound defaults (00:00 start / 23:59 end). */
function isCustomTime(t: string, isEnd: boolean): boolean {
    if (!t) return false
    return isEnd ? t !== '23:59' : t !== '00:00'
}
