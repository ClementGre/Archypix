// A horizontal timeline of reference dates with the derived (mean) date marked (feature 30 §7).

import {formatNaive} from '@/components/photos/detail/DateTimePickerPopover'
import {msToNaive} from '@/lib/fixDate'

export function DateTimelinePreview({refTimes, derived}: { refTimes: number[]; derived: number }) {
    const all = [...refTimes, derived]
    const min = Math.min(...all)
    const max = Math.max(...all)
    const span = max - min
    const pos = (t: number) => (span === 0 ? 50 : ((t - min) / span) * 100)
    return (
        <div className="px-2 py-3">
            <div className="relative h-8">
                <div className="absolute inset-x-0 top-4 h-px bg-border"/>
                {refTimes.map((t, i) => (
                    <div
                        key={i}
                        className="absolute top-4 h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-sky-400 ring-2 ring-background"
                        style={{left: `${pos(t)}%`}}
                        title={formatNaive(msToNaive(t))}
                    />
                ))}
                <div
                    className="absolute top-4 h-3.5 w-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-background bg-primary"
                    style={{left: `${pos(derived)}%`}}
                    title={`Derived: ${formatNaive(msToNaive(derived))}`}
                />
            </div>
            <div className="mt-1 flex justify-between text-[10px] text-muted-foreground">
                <span>{formatNaive(msToNaive(min))}</span>
                {span > 0 && <span>{formatNaive(msToNaive(max))}</span>}
            </div>
        </div>
    )
}
