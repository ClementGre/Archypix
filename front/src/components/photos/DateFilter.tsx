// Capture-date range filter for the grid header. The shared `DateRangePicker` is already a
// self-contained button-with-calendar-popover, so this just wires the URL params to it (no extra
// wrapping popup). Writes `capturedAfter`/`capturedBefore` (stored RFC3339 UTC; picker in YYYY-MM-DD).

import {DateRangePicker} from '@/components/common/DateRangePicker'
import {useGalleryParams} from '@/hooks/useGalleryParams'

export function DateFilter() {
    const {params, update} = useGalleryParams()
    const fromDate = params.capturedAfter?.slice(0, 10) ?? ''
    const toDate = params.capturedBefore?.slice(0, 10) ?? ''
    const active = !!params.capturedAfter || !!params.capturedBefore

    const onDateRange = (from: string, to: string) =>
        update({
            capturedAfter: from ? `${from}T00:00:00Z` : null,
            capturedBefore: to ? `${to}T23:59:59Z` : null,
        })

    return (
        <DateRangePicker
            mode="date"
            from={fromDate}
            to={toDate}
            onChange={onDateRange}
            placeholder="Date"
            // Match the other controls: grayed by default, primary once a range is set.
            triggerClassName={active ? 'border-primary/50 text-primary [&_svg]:text-primary' : undefined}
        />
    )
}
