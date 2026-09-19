import {useEffect, useMemo, useState} from 'react'
import {AlertTriangle, ArrowRight, Pencil, Trash2} from 'lucide-react'
import {toast} from 'sonner'
import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Label} from '@/components/ui/label'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {TagPicker} from '@/components/tags/TagPicker'
import {
    draftFromMeta,
    draftToPatch,
    type TagMetaDraft,
    TagMetaFields,
    TagNameField,
    toNaive,
} from '@/components/tags/TagMetaFields'
import {
    useAllTagsWithSources,
    useRenameTag,
    useResetTagMeta,
    useTagTree,
    useWriteTagMeta,
} from '@/hooks/useTags'
import {display} from '@/lib/tagTree'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import type {TagListItem} from '@/lib/types'

/**
 * The one tag dialog, reached from the tag tree's `…` menu and absorbing the old `RenameTagDialog`.
 * Fields map 1:1 to feature 34 §3 and are shared with the create dialog; every write goes through
 * the §4.1 queue except the rename (an async cascade) and *Reset metadata* (no undo, so it is
 * already confirmed).
 */
export function EditTagDialog({
                                  tagPath,
                                  open,
                                  onOpenChange,
                              }: {
    tagPath: string
    open: boolean
    onOpenChange: (open: boolean) => void
}) {
    const {items, metaByPath} = useTagTree()
    const write = useWriteTagMeta()
    const reset = useResetTagMeta()
    const rename = useRenameTag()
    const {data: sourced} = useAllTagsWithSources(open)

    const item = items.find((i) => i.path === tagPath)
    const meta = metaByPath.get(tagPath) ?? null
    const label = TagPath.leaf(tagPath)

    const [draft, setDraft] = useState<TagMetaDraft>(() => draftFromMeta(meta))
    const [newTag, setNewTag] = useState('')

    // Re-seed the form from the stored row whenever the dialog opens for a tag.
    useEffect(() => {
        if (!open) return
        setDraft(draftFromMeta(meta))
        setNewTag('')
        // The stored row is the source of truth on open; later keystrokes are local state.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open, tagPath])

    const patch = (fields: Partial<TagMetaDraft>) => setDraft((d) => ({...d, ...fields}))

    const sources = useMemo(
        () => sourced?.find((i: TagListItem) => i.path === tagPath)?.sources ?? [],
        [sourced, tagPath],
    )

    /** An empty `show_when_empty` tag has no pictures — the row *is* the tag, so the reset reads as
     *  a deletion (§6). */
    const isEmptyTag = (item?.count ?? 0) === 0 && !!meta?.show_when_empty

    const save = () => {
        write(draftToPatch(tagPath, draft))
        onOpenChange(false)
    }

    const submitRename = () => {
        rename.mutate(
            {oldTag: tagPath, newTag},
            {
                onSuccess: () => {
                    toast.success(`Renaming ${TagPath.toDisplay(tagPath)} → ${TagPath.toDisplay(newTag)}…`)
                    onOpenChange(false)
                },
                onError: (e) => toast.error(apiErrorMessage(e)),
            },
        )
    }

    const doReset = () => {
        reset.mutate([tagPath], {
            onSuccess: () => {
                toast.success(isEmptyTag ? 'Tag deleted' : 'Tag metadata reset')
                onOpenChange(false)
            },
            onError: (e) => toast.error(apiErrorMessage(e)),
        })
    }

    const isDescendant = (a: string, b: string) => b === a || b.startsWith(`${a}.`)
    const renameInvalid =
        !newTag || newTag === tagPath || isDescendant(tagPath, newTag) || isDescendant(newTag, tagPath)

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
                <DialogHeader>
                    <DialogTitle>{display(tagPath, meta)}</DialogTitle>
                    <DialogDescription className="font-mono text-xs">{TagPath.toDisplay(tagPath)}</DialogDescription>
                </DialogHeader>

                {/* Read-only header: totals, provenance and outgoing shares. */}
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                    <span>{item?.count ?? 0} photos</span>
                    <span>· {item?.exact_count ?? 0} directly</span>
                    {!!item?.trashed?.count && <span>· {item.trashed.count} in trash</span>}
                    {sources.length > 0 && (
                        <span>· {sources.map((s) => `${s.count} ${s.source}`).join(', ')}</span>
                    )}
                </div>

                <div className="space-y-4 py-1">
                    <TagNameField value={draft.name} onChange={(name) => patch({name})} placeholder={label}/>
                    <TagMetaFields
                        draft={draft}
                        onChange={patch}
                        label={label}
                        derived={{from: toNaive(item?.date_from), to: toNaive(item?.date_to)}}
                    />

                    {/* The rename control — the path is the identity, so this is the async cascade. */}
                    <div className="space-y-1.5">
                        <Label>Rename tag path</Label>
                        <div className="flex items-center gap-2 text-sm">
                            <span className="truncate rounded bg-muted px-2 py-1 font-mono text-xs">
                                {TagPath.toDisplay(tagPath)}
                            </span>
                            <ArrowRight className="h-4 w-4 shrink-0 text-muted-foreground"/>
                            <TagPicker
                                onSelect={setNewTag}
                                allowCreate
                                triggerLabel={newTag ? TagPath.toDisplay(newTag) : 'New path…'}
                                placeholder="Search or create the new tag…"
                            />
                            <Button size="sm" disabled={renameInvalid || rename.isPending} onClick={submitRename}>
                                <Pencil className="mr-1.5 h-3.5 w-3.5"/>
                                Rename
                            </Button>
                        </div>
                        <p className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                            <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                            <span>
                                Renaming rewrites the tag on your pictures and in every hierarchy, tagging service and
                                outgoing share that references it. The cascade runs in the background and cannot be
                                undone automatically.
                            </span>
                        </p>
                    </div>

                </div>

                <DialogFooter className="sm:justify-between">
                    {/* Worded by consequence, not by mechanism (§6): the confirm names what disappears. */}
                    <ConfirmDialog
                        trigger={
                            <Button variant="ghost" size="sm" className="text-destructive">
                                <Trash2 className="mr-1.5 h-3.5 w-3.5"/>
                                {isEmptyTag ? 'Delete tag' : 'Reset metadata'}
                            </Button>
                        }
                        title={isEmptyTag ? 'Delete this tag?' : 'Reset this tag’s metadata?'}
                        description={
                            isEmptyTag
                                ? 'This tag has no photos, so deleting it removes it entirely, along with its display name, description, cover and dates. There is no undo.'
                                : 'The display name, description, cover and dates are removed. The tag and its photos stay. There is no undo.'
                        }
                        confirmLabel={isEmptyTag ? 'Delete tag' : 'Reset'}
                        destructive
                        onConfirm={doReset}
                    />
                    <div className="flex gap-2">
                        <Button variant="outline" onClick={() => onOpenChange(false)}>
                            Cancel
                        </Button>
                        <Button onClick={save}>Save</Button>
                    </div>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
