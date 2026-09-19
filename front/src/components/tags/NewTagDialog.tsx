import {useEffect, useState} from 'react'
import {ChevronRight, Plus, RotateCcw} from 'lucide-react'
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
import {
    draftFromMeta,
    draftToPatch,
    type TagMetaDraft,
    TagMetaFields,
    TagNameField,
    TagPathField,
} from '@/components/tags/TagMetaFields'
import {useTagTree, useWriteTagMeta} from '@/hooks/useTags'
import {cn, TagPath} from '@/lib/utils'

/**
 * Create a tag (§6). §1 promises an event can exist before it is filled — without this the only path
 * to an empty tag is WebDAV `MKCOL`, which is not a path most users have.
 *
 * The typed name is the display name and the path is slugified from it; *Customize* opens the path
 * and the rest of the metadata so a tag arrives configured rather than created then edited. A sibling
 * slug collision is **rejected** rather than auto-disambiguated: a silent `_2` suffix is worse than
 * asking (§5).
 */
export function NewTagDialog({
                                 parentPath,
                                 initialName = '',
                                 emptyByDefault = true,
                                 open,
                                 onOpenChange,
                                 onCreated,
                             }: {
    /** `null` creates at root level. */
    parentPath: string | null
    /** Seeds the name field — the text typed into a `TagPicker`. */
    initialName?: string
    /** `show_when_empty` default: on for a deliberately empty tag, off when photos follow at once. */
    emptyByDefault?: boolean
    open: boolean
    onOpenChange: (open: boolean) => void
    /** The created wire path, for a caller that goes on to use it (e.g. assigning it to photos). */
    onCreated?: (wirePath: string) => void
}) {
    const {items} = useTagTree()
    const write = useWriteTagMeta()
    const [draft, setDraft] = useState<TagMetaDraft>(() => draftFromMeta(null, emptyByDefault))
    const [customizing, setCustomizing] = useState(false)
    /** Display-form path once the user takes it over; `null` keeps it following the name. */
    const [customPath, setCustomPath] = useState<string | null>(null)

    useEffect(() => {
        if (!open) return
        setDraft({...draftFromMeta(null, emptyByDefault), name: initialName})
        setCustomizing(false)
        setCustomPath(null)
    }, [open, parentPath, initialName, emptyByDefault])

    const patch = (fields: Partial<TagMetaDraft>) => setDraft((d) => ({...d, ...fields}))

    const typed = draft.name.trim()
    const slug = typed ? TagPath.slugify(typed) : ''
    const inferred = slug ? (parentPath ? `${parentPath}.${slug}` : slug) : ''
    // The typed path is auto-fixed on the way to the wire form; what cannot be fixed blocks Create.
    const pathInput = customPath ?? TagPath.toDisplay(inferred)
    const path = customPath === null ? inferred : TagPath.toWireSlug(TagPath.sanitize(customPath).clean)
    const badChars = customPath === null ? [] : TagPath.invalidChars(customPath)

    const collides = !!path && items.some((i) => i.path === path)
    const protectedPath = !!path && TagPath.isProtected(path)
    const invalid = !path || collides || protectedPath || badChars.length > 0

    const submit = () => {
        if (invalid) return
        write(draftToPatch(path, draft))
        toast.success(`Created ${TagPath.toDisplay(path)}`)
        onCreated?.(path)
        onOpenChange(false)
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-md">
                <DialogHeader>
                    <DialogTitle>New tag</DialogTitle>
                    <DialogDescription>
                        {parentPath
                            ? <>Created under <span className="font-mono text-xs">{TagPath.toDisplay(parentPath)}</span>.</>
                            : 'Created at the top level.'}
                        {draft.showWhenEmpty && ' It stays in the tree until you put photos in it.'}
                    </DialogDescription>
                </DialogHeader>

                <div className="space-y-4">
                    <TagNameField value={draft.name} onChange={(name) => patch({name})}
                                  placeholder="My new tag" onEnter={submit}/>

                    {path && (
                        <p className="text-[11px] text-muted-foreground">
                            Path: <span className="font-mono">{TagPath.toDisplay(path)}</span>
                        </p>
                    )}
                    {collides && <p className="text-[11px] text-destructive">That tag already exists.</p>}
                    {protectedPath && (
                        <p className="text-[11px] text-destructive">“SharedToMe” is a reserved prefix and can’t be used.</p>
                    )}

                    <button
                        type="button"
                        onClick={() => setCustomizing((v) => !v)}
                        className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
                    >
                        <ChevronRight className={cn('h-3.5 w-3.5 transition-transform', customizing && 'rotate-90')}/>
                        Customize
                    </button>

                    {customizing && (
                        <div className="space-y-4 border-l-2 border-border pl-3">
                            <TagPathField value={pathInput} onChange={setCustomPath}>
                                {customPath === null ? (
                                    <p className="text-[11px] text-muted-foreground">
                                        Follows the display name — “/” separates levels.
                                    </p>
                                ) : (
                                    <button
                                        type="button"
                                        onClick={() => setCustomPath(null)}
                                        className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
                                    >
                                        <RotateCcw className="h-3 w-3"/>
                                        Follow the display name again
                                    </button>
                                )}
                            </TagPathField>
                            <TagMetaFields
                                draft={draft}
                                onChange={patch}
                                label={TagPath.leaf(path) || 'tag'}
                            />
                        </div>
                    )}
                </div>

                <DialogFooter>
                    <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
                    <Button onClick={submit} disabled={invalid}>
                        <Plus className="mr-1.5 h-3.5 w-3.5"/>
                        Create
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
