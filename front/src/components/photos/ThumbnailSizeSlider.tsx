import {LayoutGrid} from 'lucide-react'
import {ROW_HEIGHT_MAX, ROW_HEIGHT_MIN, useUIStore} from '@/stores/ui'

/** The thumbnail-size (grid zoom) slider — shared by the app footer and the public share footer. */
export function ThumbnailSizeSlider() {
    const rowHeight = useUIStore((s) => s.rowHeight)
    const setRowHeight = useUIStore((s) => s.setRowHeight)
    return (
        <label className="flex items-center gap-1.5" title="Thumbnail size">
            <LayoutGrid className="h-3 w-3"/>
            <input
                type="range"
                min={ROW_HEIGHT_MIN}
                max={ROW_HEIGHT_MAX}
                step={10}
                value={rowHeight}
                onChange={(e) => setRowHeight(Number(e.target.value))}
                className="h-1 w-30 cursor-pointer accent-primary"
                aria-label="Thumbnail size"
            />
        </label>
    )
}
