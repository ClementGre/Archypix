import {useEffect, useMemo, useState} from 'react'
import {AlertTriangle, ArrowRight, Check, ImageIcon, Pencil, RotateCcw, Trash2} from 'lucide-react'
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
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {Textarea} from '@/components/ui/textarea'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {TagPicker} from '@/components/tags/TagPicker'
import {
    useAllTagsWithSources,
    useRenameTag,
    useResetTagMeta,
    useTagTree,
    useWriteTagMeta,
} from '@/hooks/useTags'
import {display} from '@/lib/tagTree'
import {apiErrorMessage} from '@/api/client'
import {cn, TagPath} from '@/lib/utils'
import type {TagListItem, TagMetaPatch} from '@/lib/types'

/** A small fixed palette; the picker behind "Custom" stores free hex (§3.2). */
const PALETTE = [
    '#ef4444', '#f97316', '#eab308', '#22c55e',
    '#14b8a6', '#3b82f6', '#8b5cf6', '#ec4899',
]

/** `datetime-local` wants `YYYY-MM-DDTHH:mm`; the API speaks naive-UTC timestamps. */
function toLocalInput(iso: string | null | undefined): string {
    return iso ? iso.replace(' ', 'T').slice(0, 16) : ''
}

function fromLocalInput(value: string): string | null {
    return value ? `${value}:00` : null
}

/**
 * The one tag dialog, reached from the tag tree's `…` menu and absorbing the old `RenameTagDialog`.
 * Fields map 1:1 to feature 34 §3; every write goes through the §4.1 queue except the rename
 * (an async cascade) and *Reset metadata* (no undo, so it is already confirmed).
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

    const [name, setName] = useState('')
    const [description, setDescription] = useState('')
    const [color, setColor] = useState('')
    const [dateFrom, setDateFrom] = useState('')
    const [dateTo, setDateTo] = useState('')
    const [showWhenEmpty, setShowWhenEmpty] = useState(false)
    const [webdavDirName, setWebdavDirName] = useState('')
    const [newTag, setNewTag] = useState('')

    // Re-seed the form from the stored row whenever the dialog opens for a tag.
    useEffect(() => {
        if (!open) return
        setName(meta?.display_name ?? '')
        setDescription(meta?.description ?? '')
        setColor(meta?.color ?? '')
        setDateFrom(toLocalInput(meta?.date_from))
        setDateTo(toLocalInput(meta?.date_to))
        setShowWhenEmpty(meta?.show_when_empty ?? false)
        setWebdavDirName(meta?.webdav_dir_name ?? '')
        setNewTag('')
        // The stored row is the source of truth on open; later keystrokes are local state.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open, tagPath])

    const sources = useMemo(
        () => sourced?.find((i: TagListItem) => i.path === tagPath)?.sources ?? [],
        [sourced, tagPath],
    )

    /** An empty `show_when_empty` tag has no pictures — the row *is* the tag, so the reset reads as
     *  a deletion (§6). */
    const isEmptyTag = (item?.count ?? 0) === 0 && !!meta?.show_when_empty

    const save = () => {
        const patch: TagMetaPatch = {
            tag_path: tagPath,
            display_name: name.trim() || null,
            description: description.trim() || null,
            color: color || null,
            date_from: fromLocalInput(dateFrom),
            date_to: fromLocalInput(dateTo),
            show_when_empty: showWhenEmpty,
            webdav_dir_name: webdavDirName.trim() || null,
        }
        write(patch)
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
                    <div className="space-y-1.5">
                        <Label htmlFor="tag-display-name">Display name</Label>
                        <Input
                            id="tag-display-name"
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            placeholder={label}
                            maxLength={128}
                        />
                        <p className="text-[11px] text-muted-foreground">
                            Emoji are welcome — the name travels everywhere the tag does.
                        </p>
                    </div>

                    <div className="space-y-1.5">
                        <Label htmlFor="tag-description">Description</Label>
                        <Textarea
                            id="tag-description"
                            value={description}
                            onChange={(e) => setDescription(e.target.value)}
                            rows={2}
                            maxLength={2000}
                        />
                    </div>

                    <div className="space-y-1.5">
                        <Label>Colour</Label>
                        <div className="flex flex-wrap items-center gap-1.5">
                            {PALETTE.map((c) => (
                                <button
                                    key={c}
                                    type="button"
                                    onClick={() => setColor(color === c ? '' : c)}
                                    style={{backgroundColor: c}}
                                    className={cn(
                                        'flex h-6 w-6 items-center justify-center rounded-full border',
                                        color === c ? 'ring-2 ring-offset-1 ring-ring' : 'border-transparent',
                                    )}
                                    aria-label={`Colour ${c}`}
                                >
                                    {color === c && <Check className="h-3 w-3 text-white"/>}
                                </button>
                            ))}
                            <Input
                                type="color"
                                value={color || '#888888'}
                                onChange={(e) => setColor(e.target.value)}
                                className="h-6 w-10 cursor-pointer p-0.5"
                                aria-label="Custom colour"
                            />
                            {color && (
                                <Button variant="ghost" size="sm" className="h-6 px-2 text-xs"
                                        onClick={() => setColor('')}>
                                    Clear
                                </Button>
                            )}
                        </div>
                    </div>

                    {/* Placeholders show the derived range; clearing a side resets it to derived (§4). */}
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <Label htmlFor="tag-date-from">From</Label>
                            <Input
                                id="tag-date-from"
                                type="datetime-local"
                                value={dateFrom}
                                onChange={(e) => setDateFrom(e.target.value)}
                                placeholder={toLocalInput(item?.date_from)}
                            />
                            <button
                                type="button"
                                onClick={() => setDateFrom('')}
                                className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
                            >
                                <RotateCcw className="h-3 w-3"/>
                                Derived: {toLocalInput(item?.date_from) || '—'}
                            </button>
                        </div>
                        <div className="space-y-1.5">
                            <Label htmlFor="tag-date-to">To</Label>
                            <Input
                                id="tag-date-to"
                                type="datetime-local"
                                value={dateTo}
                                onChange={(e) => setDateTo(e.target.value)}
                                placeholder={toLocalInput(item?.date_to)}
                            />
                            <button
                                type="button"
                                onClick={() => setDateTo('')}
                                className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
                            >
                                <RotateCcw className="h-3 w-3"/>
                                Derived: {toLocalInput(item?.date_to) || '—'}
                            </button>
                        </div>
                    </div>

                    <div className="flex items-center justify-between gap-3">
                        <div>
                            <Label htmlFor="tag-show-empty">Show when empty</Label>
                            <p className="text-[11px] text-muted-foreground">
                                Keep the tag in the tree even with no photos.
                            </p>
                        </div>
                        <Switch id="tag-show-empty" checked={showWhenEmpty} onCheckedChange={setShowWhenEmpty}/>
                    </div>

                    <div className="space-y-1.5">
                        <Label htmlFor="tag-webdav">WebDAV folder name</Label>
                        <div className="flex items-center gap-2">
                            <Input
                                id="tag-webdav"
                                value={webdavDirName}
                                onChange={(e) => setWebdavDirName(e.target.value)}
                                placeholder={label}
                                maxLength={255}
                            />
                            <Button
                                variant="outline"
                                size="sm"
                                disabled={!name.trim()}
                                onClick={() => setWebdavDirName(name.trim())}
                            >
                                Use display name
                            </Button>
                        </div>
                        <p className="text-[11px] text-muted-foreground">
                            Set once, deliberately — a mounted client sees a folder rename as delete + create, so
                            this never follows the display name on its own.
                        </p>
                    </div>

                    {/* The rename control — the path is the identity, so this is the async cascade. */}
                    <div className="space-y-1.5 rounded-md border p-3">
                        <Label>Path</Label>
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

                    {!!meta?.cover_picture_id && (
                        <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                            <ImageIcon className="h-3 w-3"/>
                            A cover photo is set.
                            <button
                                type="button"
                                className="underline"
                                onClick={() => write({tag_path: tagPath, cover_picture_id: null})}
                            >
                                Clear
                            </button>
                        </p>
                    )}
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
