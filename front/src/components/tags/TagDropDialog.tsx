import {useEffect, useState} from 'react'
import {toast} from 'sonner'
import {
    AlertDialog,
    AlertDialogAction,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import {Button} from '@/components/ui/button'
import {batchEditTags} from '@/api/tags'
import {useBatchEditTags} from '@/hooks/useTags'
import {apiErrorMessage} from '@/api/client'
import {areSiblings} from '@/stores/tagDrag'
import type {BatchDryRun, PictureSelection} from '@/lib/types'

export interface TagDrop {
    selection: PictureSelection
    count: number
    sourceTag: string | null
    targetTag: string
    /** Display name of the target, for the wording. */
    targetName: string
    /** Display name of the source, when there is one. */
    sourceName: string | null
}

/**
 * The drop confirmation (feature 34 §9). A single photo onto a non-sibling tag is assigned
 * silently by the caller; everything else lands here, with real figures from
 * `batchEditTags({dry_run: true})` rather than a raw count.
 */
export function TagDropDialog({drop, onClose}: { drop: TagDrop | null; onClose: () => void }) {
    const edit = useBatchEditTags()
    const [dry, setDry] = useState<BatchDryRun | null>(null)

    useEffect(() => {
        setDry(null)
        if (!drop) return
        let live = true
        void batchEditTags({selection: drop.selection, add_tags: [drop.targetTag], dry_run: true})
            .then((d) => live && setDry(d))
            .catch(() => undefined)
        return () => {
            live = false
        }
    }, [drop])

    if (!drop) return null
    const sibling = !!drop.sourceTag && areSiblings(drop.sourceTag, drop.targetTag)

    const apply = (alsoRemoveSource: boolean) => {
        const remove = alsoRemoveSource && drop.sourceTag ? [drop.sourceTag] : []
        edit.mutate(
            {selection: drop.selection, add_tags: [drop.targetTag], remove_tags: remove},
            {
                onSuccess: () => toast.success(`Added to ${drop.targetName}`, undoAction(drop, dry, edit)),
                onError: (e) => toast.error(apiErrorMessage(e)),
            },
        )
        onClose()
    }

    // `count` is 0 for a select-all over a query — the dry run is the authority either way (§9).
    const total = dry?.affected ?? drop.count
    const added = dry?.added ?? total

    return (
        <AlertDialog open onOpenChange={(o) => !o && onClose()}>
            <AlertDialogContent>
                <AlertDialogHeader>
                    <AlertDialogTitle>
                        Add {total} photo{total === 1 ? '' : 's'} to {drop.targetName}?
                    </AlertDialogTitle>
                    <AlertDialogDescription>
                        {dry
                            ? `${added} will gain the tag${added < total ? `; ${total - added} already have it` : ''}.`
                            : 'Checking…'}
                        {sibling && drop.sourceName && ` They are currently in ${drop.sourceName}.`}
                    </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                    {sibling && drop.sourceName ? (
                        <>
                            <Button variant="outline" onClick={() => apply(false)}>Add only</Button>
                            <AlertDialogAction onClick={() => apply(true)}>
                                Add and remove {drop.sourceName}
                            </AlertDialogAction>
                        </>
                    ) : (
                        <AlertDialogAction onClick={() => apply(false)}>Add</AlertDialogAction>
                    )}
                </AlertDialogFooter>
            </AlertDialogContent>
        </AlertDialog>
    )
}

/**
 * Undo is scoped to the pictures that actually **gained** the tag (§9) — stripping it from ones
 * that already carried it would be destructive. The dry run reports how many gained it, not which,
 * so the undo is offered only when every selected picture gained it; otherwise the toast says what
 * happened and leaves the removal to the batch panel.
 */
export function undoAction(
    drop: TagDrop,
    dry: BatchDryRun | null,
    edit: ReturnType<typeof useBatchEditTags>,
) {
    const added = dry?.added
    const total = dry?.affected ?? drop.count
    if (added != null && added < total) {
        return {description: `${added} of ${total} gained the tag; the rest already had it.`}
    }
    return {
        action: {
            label: 'Undo',
            onClick: () =>
                edit.mutate({selection: drop.selection, remove_tags: [drop.targetTag]}),
        },
    }
}
