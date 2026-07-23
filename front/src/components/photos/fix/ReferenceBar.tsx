// Floating banner shown during the reference-picking phase (feature 30 §7/§13): it replaces the
// normal SelectionActionBar so batch trash/tag actions can't fire against the reference selection.
// The derived value + Apply live in the details panel; on mobile (no right-panel toggle) the Review
// button reopens the drawer to reach them.

import {PanelRight, Users, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {useFixReference} from '@/stores/fixReference'
import {useReferencePhase} from '@/hooks/useReferencePhase'
import {useSelectionStore} from '@/stores/selection'
import {useUIStore} from '@/stores/ui'
import {useIsMobile} from '@/hooks/useMediaQuery'

export function ReferenceBar() {
    const {active, field, targetIds, entrySig} = useFixReference()
    const {exit} = useReferencePhase()
    const queueLand = useSelectionStore((s) => s.queueLand)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const isMobile = useIsMobile()
    if (!active) return null

    // Cancelling keeps a single target selected once the restored view is back on screen (§8); a batch
    // just exits. `destSig` binds the intent to that view so PhotoGrid doesn't resolve it too early.
    const cancel = () => {
        if (targetIds.length === 1) queueLand({anchorId: targetIds[0], advance: false, destSig: entrySig})
        exit()
    }

    return (
        <div className="pointer-events-none fixed inset-x-0 bottom-4 z-30 flex justify-center px-4">
            <div
                className="pointer-events-auto flex items-center gap-3 rounded-full border border-primary/40 bg-background/95 px-4 py-2 text-sm shadow-lg backdrop-blur">
                <Users className="h-4 w-4 text-primary"/>
                <span>
                    Pick {!isMobile && (field === 'gps' ? 'location' : 'date')} references
                </span>
                {isMobile ? (
                    <Button variant="secondary" size="sm" className="h-7 gap-1 text-xs" onClick={() => openMobileDrawer('right')}>
                        <PanelRight className="h-3.5 w-3.5"/> Review
                    </Button>
                ) : (
                    <span className="text-xs text-muted-foreground">Apply from the side panel</span>
                )}
                <Button variant="ghost" size="sm" className="h-7 gap-1 text-xs" onClick={cancel}>
                    <X className="h-3.5 w-3.5"/> Cancel
                </Button>
            </div>
        </div>
    )
}
