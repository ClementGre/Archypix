import {useCallback} from 'react'
import {useSearchParams} from 'react-router-dom'
import {useFixReference} from '@/stores/fixReference'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useUIStore} from '@/stores/ui'
import type {FixMode} from '@/lib/types'

/**
 * Enter/exit the fix-tools reference-picking phase (feature 30 §7) while snapshotting and restoring
 * the gallery's filters/sort: the phase lets the user freely change tag/sort/date filters to find
 * references, then puts the view back the way it was on exit (keeping `fix` mode on).
 */
export function useReferencePhase() {
    const [sp, setSp] = useSearchParams()
    const {selectionFilter} = useGalleryParams()
    const enter = useFixReference((s) => s.enter)
    const cancel = useFixReference((s) => s.cancel)
    const closeMobileDrawer = useUIStore((s) => s.closeMobileDrawer)

    const begin = useCallback(
        (field: FixMode, targetIds: string[]) => {
            // Snapshot both the URL (to restore) and the view's selection-filter signature (so the
            // deferred land intent knows when this exact view is back on screen after exit).
            enter(field, targetIds, sp.toString(), JSON.stringify(selectionFilter))
            // On mobile the sidebar is an overlay drawer over the grid — close it so the user can tap
            // reference photos; they reopen it (to preview/apply) via the reference bar's Review button.
            closeMobileDrawer()
        },
        [enter, sp, selectionFilter, closeMobileDrawer],
    )

    // Leave the phase and restore the pre-phase filters. `overrideFix` (when passed) also sets/clears
    // the `fix` param in the same navigation — used when the user switches or turns off fix mode from
    // the dropdown mid-phase (a single setSearchParams avoids racing two calls).
    const exit = useCallback(
        (overrideFix?: FixMode | null) => {
            const saved = useFixReference.getState().savedSearch
            cancel()
            const next = new URLSearchParams(saved ?? sp.toString())
            if (overrideFix !== undefined) {
                if (overrideFix) next.set('fix', overrideFix)
                else next.delete('fix')
            }
            setSp(next)
        },
        [cancel, setSp, sp],
    )

    return {begin, exit}
}
