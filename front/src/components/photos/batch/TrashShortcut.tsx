import {useEffect, useMemo, useState} from 'react'
import {useSearchParams} from 'react-router-dom'
import {useTrashMutations} from '@/hooks/usePictureEdit'
import {toApiSelection, useSelectionStore} from '@/stores/selection'
import {useFixReference} from '@/stores/fixReference'
import {useGridItems} from '@/stores/gridItems'
import {isTrashShortcut} from '@/lib/utils'
import {BatchTrashDialog} from './BatchTrashDialog'

/**
 * Delete / ⌘·Ctrl+Backspace on the gallery selection: a single picture is trashed directly (a
 * deliberate shortcut needs no confirm), a multi-selection opens the batch confirm.
 */
export function TrashShortcut() {
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const [sp] = useSearchParams()
    const {trash} = useTrashMutations()
    const [confirmOpen, setConfirmOpen] = useState(false)

    const selection = useMemo(() => toApiSelection({query, includeIds, excludeIds}), [query, includeIds, excludeIds])
    const single = query === null && includeIds.length === 1 && excludeIds.length === 0
    const hasSelection = query !== null || includeIds.length > 0

    // The Lightbox handles the shortcut itself while open; the reference phase's selection is not a target set.
    useEffect(() => {
        if (!hasSelection || sp.has('view')) return
        const onKey = (e: KeyboardEvent) => {
            const t = e.target as HTMLElement | null
            if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
            if (!isTrashShortcut(e) || useFixReference.getState().active) return
            if (!single) {
                e.preventDefault()
                setConfirmOpen(true)
                return
            }
            const id = includeIds[0]
            if (useGridItems.getState().items.find((i) => i.id === id)?.deleted_at) return
            e.preventDefault()
            trash.mutate(id)
        }
        window.addEventListener('keydown', onKey)
        return () => window.removeEventListener('keydown', onKey)
    }, [hasSelection, single, sp, includeIds, trash])

    return <BatchTrashDialog selection={selection} open={confirmOpen} onOpenChange={setConfirmOpen}/>
}
