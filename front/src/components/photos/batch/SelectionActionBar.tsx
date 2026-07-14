import {CheckCheck, FlipHorizontal2, Loader2, SlidersHorizontal, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useSelectionCount} from '@/hooks/useAggregate'
import {useIsMobile} from '@/hooks/useMediaQuery'

/**
 * Floating selection bar (§7): shown on desktop **and** mobile whenever more than one picture is
 * selected. Carries the resolved count, Select-all (adopts the view's query), Invert, Clear, and a
 * Batch-actions button that surfaces the right (multi-select) panel.
 *
 * `onSelectAll`/`onInvert` override the default query-mode behaviour — the token-gated public share
 * page has no `PictureFilter`, so it passes explicit select-all/invert over the loaded ids instead.
 */
export function SelectionActionBar({onSelectAll, onInvert}: { onSelectAll?: () => void; onInvert?: () => void } = {}) {
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const clear = useSelectionStore((s) => s.clear)
    const invert = useSelectionStore((s) => s.invert)
    const selectAll = useSelectionStore((s) => s.selectAll)

    const setRightOpen = useUIStore((s) => s.setRightOpen)
    const rightSidebarOpen = useUIStore((s) => s.rightSidebarOpen)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const mobileDrawer = useUIStore((s) => s.mobileDrawer)
    const isMobile = useIsMobile()

    const {selectionFilter} = useGalleryParams()
    const {count, loading} = useSelectionCount()

    const doSelectAll = onSelectAll ?? (() => selectAll(selectionFilter))
    const doInvert = onInvert ?? (() => invert(selectionFilter))

    // Show only for a genuine multi-selection. A single explicit picture uses the detail panel.
    const isSingle = query === null && includeIds.length === 1 && excludeIds.length === 0
    const hasSelection = query !== null || includeIds.length > 0
    if (!hasSelection || isSingle) return null
    // On mobile, hide while the drawer is open so it doesn't sit under the overlay.
    if (isMobile && mobileDrawer !== null) return null

    const openBatch = () => {
        if (isMobile) openMobileDrawer('right')
        else setRightOpen(true)
    }
    // On desktop the right panel is already docked open when `rightSidebarOpen` — no need for the
    // button. On mobile (drawer model) it's always useful.
    const showBatchButton = isMobile || !rightSidebarOpen

    return (
        // The full-width container is click-through (`pointer-events-none`); only the pill catches clicks.
        <div className="pointer-events-none fixed inset-x-0 bottom-4 z-40 flex justify-center px-4">
            <div className="pointer-events-auto flex items-center gap-1 rounded-full border border-border bg-card/95 p-1 shadow-lg backdrop-blur">
                <span className="flex items-center gap-1 px-2 text-sm font-medium tabular-nums">
                    {loading ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : count}
                    <span className="hidden font-normal text-muted-foreground sm:inline">selected</span>
                </span>
                <Button variant="ghost" size="sm" className="gap-1.5 rounded-full" onClick={doSelectAll}>
                    <CheckCheck className="h-4 w-4"/> Select all
                </Button>
                {
                    <Button variant="ghost" size="sm" className="gap-1.5 rounded-full" onClick={doInvert}
                            title="Invert selection">
                        <FlipHorizontal2 className="h-4 w-4"/>
                        <span className="hidden sm:inline">Invert</span>
                    </Button>
                }
                {showBatchButton && (
                    <Button size="sm" className="gap-1.5 rounded-full" onClick={openBatch}>
                        <SlidersHorizontal className="h-4 w-4"/>
                        <span className="hidden sm:inline">Batch actions</span>
                    </Button>
                )}
                <Button variant="ghost" size="icon" className="h-8 w-8 rounded-full" onClick={clear} aria-label="Clear selection">
                    <X className="h-4 w-4"/>
                </Button>
            </div>
        </div>
    )
}
