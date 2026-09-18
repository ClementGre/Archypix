import {useEffect, useState} from 'react'
import {Plus} from 'lucide-react'
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
import {useTagTree, useWriteTagMeta} from '@/hooks/useTags'
import {TagPath} from '@/lib/utils'

/**
 * Coerce a typed label into a valid ltree label — the client-side twin of the backend's
 * `TagPath::slugify_label`, so `Vietnam 🇻🇳 2024` mints `Vietnam_2024` and keeps what was typed as
 * the display name (feature 34 §5).
 */
export function slugifyLabel(raw: string): string {
    let out = ''
    let prevUnderscore = false
    for (const ch of raw.normalize('NFD').replace(/[̀-ͯ]/g, '')) {
        if (/[A-Za-z0-9_]/.test(ch)) {
            out += ch
            prevUnderscore = ch === '_'
        } else if (!prevUnderscore) {
            out += '_'
            prevUnderscore = true
        }
    }
    const trimmed = out.replace(/^_+|_+$/g, '')
    return trimmed || 'untitled'
}

/**
 * Create an empty tag (§6). §1 promises an event can exist before it is filled — without this the
 * only path to an empty tag is WebDAV `MKCOL`, which is not a path most users have.
 *
 * A sibling slug collision is **rejected** rather than auto-disambiguated: a silent `_2` suffix is
 * worse than asking (§5).
 */
export function NewTagDialog({
                                 parentPath,
                                 open,
                                 onOpenChange,
                             }: {
    /** `null` creates at root level. */
    parentPath: string | null
    open: boolean
    onOpenChange: (open: boolean) => void
}) {
    const {items} = useTagTree()
    const write = useWriteTagMeta()
    const [label, setLabel] = useState('')

    useEffect(() => {
        if (open) setLabel('')
    }, [open, parentPath])

    const slug = label.trim() ? slugifyLabel(label.trim()) : ''
    const path = slug ? (parentPath ? `${parentPath}.${slug}` : slug) : ''
    const collides = !!path && items.some((i) => i.path === path)

    const submit = () => {
        write({
            tag_path: path,
            show_when_empty: true,
            display_name: label.trim() !== slug ? label.trim() : null,
        })
        toast.success(`Created ${TagPath.toDisplay(path)}`)
        onOpenChange(false)
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="sm:max-w-md">
                <DialogHeader>
                    <DialogTitle>New tag</DialogTitle>
                    <DialogDescription>
                        {parentPath
                            ? <>Created under <span className="font-mono text-xs">{TagPath.toDisplay(parentPath)}</span>.</>
                            : 'Created at the top level.'}
                        {' '}It stays in the tree until you put photos in it.
                    </DialogDescription>
                </DialogHeader>

                <div className="space-y-1.5">
                    <Label htmlFor="new-tag-name">Name</Label>
                    <Input
                        id="new-tag-name"
                        value={label}
                        onChange={(e) => setLabel(e.target.value)}
                        placeholder="Vietnam 🇻🇳 2024"
                        maxLength={128}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter' && slug && !collides) submit()
                        }}
                    />
                    {slug && (
                        <p className="text-[11px] text-muted-foreground">
                            Path: <span className="font-mono">{TagPath.toDisplay(path)}</span>
                        </p>
                    )}
                    {collides && <p className="text-[11px] text-destructive">That tag already exists.</p>}
                </div>

                <DialogFooter>
                    <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
                    <Button onClick={submit} disabled={!slug || collides}>
                        <Plus className="mr-1.5 h-3.5 w-3.5"/>
                        Create
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
