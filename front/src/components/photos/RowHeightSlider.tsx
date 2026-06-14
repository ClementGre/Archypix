import {LayoutGrid} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {ROW_HEIGHT_MAX, ROW_HEIGHT_MIN, useUIStore} from '@/stores/ui'

/** Popover with a slider that customizes the grid's baseline row height. */
export function RowHeightSlider() {
    const rowHeight = useUIStore((s) => s.rowHeight)
    const setRowHeight = useUIStore((s) => s.setRowHeight)

    return (
        <Popover>
            <PopoverTrigger asChild>
                <Button variant="ghost" size="icon" aria-label="Thumbnail size">
                    <LayoutGrid className="h-4 w-4"/>
                </Button>
            </PopoverTrigger>
            <PopoverContent align="end" className="w-56">
                <label className="mb-2 block text-sm font-medium" htmlFor="row-height">
                    Thumbnail size
                </label>
                <input
                    id="row-height"
                    type="range"
                    min={ROW_HEIGHT_MIN}
                    max={ROW_HEIGHT_MAX}
                    step={10}
                    value={rowHeight}
                    onChange={(e) => setRowHeight(Number(e.target.value))}
                    className="w-full accent-primary"
                />
                <div className="mt-1 text-right text-xs text-muted-foreground">{rowHeight}px</div>
            </PopoverContent>
        </Popover>
    )
}
