import type {ReactNode} from 'react'
import {batchTrash} from '@/api/pictures'
import {useBatchMutations} from '@/hooks/useBatch'
import {useSelectionStore} from '@/stores/selection'
import type {PictureSelection} from '@/lib/types'
import {BatchConfirmDialog} from './BatchConfirmDialog'

/** The batch move-to-trash confirm, shared by the multi-selection panel and the Delete shortcut. */
export function BatchTrashDialog({selection, trigger, open, onOpenChange}: {
    selection: PictureSelection
    trigger?: ReactNode
    open?: boolean
    onOpenChange?: (open: boolean) => void
}) {
    const clear = useSelectionStore((s) => s.clear)
    const {trash} = useBatchMutations()

    return (
        <BatchConfirmDialog
            trigger={trigger}
            open={open}
            onOpenChange={onOpenChange}
            title="Move selection to trash?"
            description="Owned photos are purged after your retention window; received photos are only hidden locally."
            confirmLabel="Move to trash"
            destructive
            dryRun={() => batchTrash(selection, true)}
            onConfirm={() => trash.mutate(selection, {onSuccess: clear})}
        />
    )
}
