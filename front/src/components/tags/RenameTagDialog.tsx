import {useEffect, useState} from 'react'
import {AlertTriangle, ArrowRight, Pencil} from 'lucide-react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {TagPicker} from '@/components/tags/TagPicker'
import {useRenameTag} from '@/hooks/useTags'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'

/**
 * Rename a tag subtree (edge case §7). Pick the new path with the tag selector, then confirm — the
 * dialog spells out that the cascade rewrites hierarchies, tagging services, and shares. Both paths
 * are wire form; `open`/`onOpenChange` are owned by the caller (the tag row `…` menu).
 */
export function RenameTagDialog({
                                    oldTag,
                                    open,
                                    onOpenChange,
                                }: {
    oldTag: string
    open: boolean
    onOpenChange: (open: boolean) => void
}) {
    const [newTag, setNewTag] = useState('')
    const rename = useRenameTag()

    // Reset the chosen target whenever the dialog (re)opens for a tag.
    useEffect(() => {
        if (open) setNewTag('')
    }, [open, oldTag])

    const isDescendant = (a: string, b: string) => b === a || b.startsWith(`${a}.`)
    const invalid =
        !newTag || newTag === oldTag || isDescendant(oldTag, newTag) || isDescendant(newTag, oldTag)

    const submit = () => {
        rename.mutate(
            {oldTag, newTag},
            {
                onSuccess: () => {
                    toast.success(`Renaming ${TagPath.toDisplay(oldTag)} → ${TagPath.toDisplay(newTag)}…`)
                    onOpenChange(false)
                },
                onError: (e) => toast.error(apiErrorMessage(e)),
            },
        )
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="sm:max-w-md">
                <DialogHeader>
                    <DialogTitle>Rename tag</DialogTitle>
                    <DialogDescription>
                        Choose a new path for <span className="font-medium">{TagPath.toDisplay(oldTag)}</span> and
                        everything under it.
                    </DialogDescription>
                </DialogHeader>

                <div className="flex items-center gap-2 text-sm">
                    <span className="truncate rounded bg-muted px-2 py-1 font-mono text-xs">
                        {TagPath.toDisplay(oldTag)}
                    </span>
                    <ArrowRight className="h-4 w-4 shrink-0 text-muted-foreground"/>
                    <TagPicker
                        onSelect={setNewTag}
                        allowCreate
                        triggerLabel={newTag ? TagPath.toDisplay(newTag) : 'New path…'}
                        placeholder="Search or create the new tag…"
                    />
                </div>

                <div
                    className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
                    <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
                    <span>
                        The tag is rewritten on your pictures and in every hierarchy, tagging service, and outgoing share that references it.
                        Segmentation tagging services’ templates are not renamed and should be updated manually.
                        This cannot be undone automatically.
                    </span>
                </div>

                <DialogFooter>
                    <Button variant="outline" onClick={() => onOpenChange(false)}>
                        Cancel
                    </Button>
                    <Button onClick={submit} disabled={invalid || rename.isPending}>
                        <Pencil className="mr-1.5 h-3.5 w-3.5"/>
                        Rename
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
