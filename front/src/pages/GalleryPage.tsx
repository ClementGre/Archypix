import {LeftPanel} from '@/components/layout/LeftPanel'
import {PhotoGrid} from '@/components/photos/PhotoGrid'
import {SelectionPanel} from '@/components/photos/SelectionPanel'
import {useUIStore} from '@/stores/ui'

/**
 * The main workspace: the unified top bar (rendered by AppShell) drives search,
 * filters, the row-height slider and the sidebar toggles; here we lay out the
 * three panes — tabbed left panel, photo grid, selection/detail panel — showing
 * a side panel only when its toggle is on.
 */
export default function GalleryPage() {
    const leftOpen = useUIStore((s) => s.leftSidebarOpen)
    const rightOpen = useUIStore((s) => s.rightSidebarOpen)

    return (
        <div className="flex h-full min-h-0">
            {leftOpen && <LeftPanel/>}
            <div className="min-w-0 flex-1">
                <PhotoGrid/>
            </div>
            {rightOpen && <SelectionPanel/>}
        </div>
    )
}
