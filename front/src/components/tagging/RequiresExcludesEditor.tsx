import {useState} from 'react'
import {Save, Undo2, X} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {TagPicker} from '@/components/tags/TagPicker'
import {TagPath} from '@/lib/utils'

interface RequiresExcludesEditorProps {
    requires: string[]
    excludes: string[]
    onUpdate: (update: { requires?: string[]; excludes?: string[] }) => void
    isPending: boolean
}

/**
 * Edits the requires/excludes gates as a local draft, committed only on Save —
 * so a multi-chip change runs the tagging pipeline once, not per chip.
 */
export function RequiresExcludesEditor({requires, excludes, onUpdate, isPending}: RequiresExcludesEditorProps) {
    const [draftRequires, setDraftRequires] = useState(requires)
    const [draftExcludes, setDraftExcludes] = useState(excludes)

    // Resync drafts whenever the persisted values change (after save / external update).
    const serverKey = `${requires.join('|')}__${excludes.join('|')}`
    const [syncedKey, setSyncedKey] = useState(serverKey)
    if (serverKey !== syncedKey) {
        setDraftRequires(requires)
        setDraftExcludes(excludes)
        setSyncedKey(serverKey)
    }

    const dirty = draftRequires.join('|') !== requires.join('|') || draftExcludes.join('|') !== excludes.join('|')

    const save = () => onUpdate({requires: draftRequires, excludes: draftExcludes})
    const reset = () => {
        setDraftRequires(requires)
        setDraftExcludes(excludes)
    }

    return (
        <div className="space-y-2 text-sm">
            <div className="flex flex-wrap items-center gap-1.5">
        <span className="w-16 shrink-0 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Requires
        </span>
                {draftRequires.map((tag) => (
                    <Badge
                        key={tag}
                        variant="secondary"
                        className="gap-1 bg-emerald-500/10 pr-1 text-emerald-600 dark:text-emerald-400"
                    >
                        {TagPath.toDisplay(tag)}
                        <button
                            onClick={() => setDraftRequires(draftRequires.filter((t) => t !== tag))}
                            className="ml-0.5 rounded-full p-0.5 hover:bg-emerald-500/20"
                        >
                            <X className="h-2.5 w-2.5"/>
                        </button>
                    </Badge>
                ))}
                <TagPicker
                    onSelect={(wire) => !draftRequires.includes(wire) && setDraftRequires([...draftRequires, wire])}
                    excludePaths={[...draftRequires, ...draftExcludes]}
                    allowCreate={false}
                    allowProtected
                    triggerLabel="Add requires"
                />
            </div>

            <div className="flex flex-wrap items-center gap-1.5">
        <span className="w-16 shrink-0 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Excludes
        </span>
                {draftExcludes.map((tag) => (
                    <Badge
                        key={tag}
                        variant="secondary"
                        className="gap-1 bg-red-500/10 pr-1 text-red-600 dark:text-red-400"
                    >
                        {TagPath.toDisplay(tag)}
                        <button
                            onClick={() => setDraftExcludes(draftExcludes.filter((t) => t !== tag))}
                            className="ml-0.5 rounded-full p-0.5 hover:bg-red-500/20"
                        >
                            <X className="h-2.5 w-2.5"/>
                        </button>
                    </Badge>
                ))}
                <TagPicker
                    onSelect={(wire) => !draftExcludes.includes(wire) && setDraftExcludes([...draftExcludes, wire])}
                    excludePaths={[...draftRequires, ...draftExcludes]}
                    allowCreate={false}
                    allowProtected
                    triggerLabel="Add excludes"
                />
            </div>

            {dirty && (
                <div className="flex items-center gap-2 pt-1">
                    <Button size="sm" className="h-7 gap-1.5" onClick={save} disabled={isPending}>
                        <Save className="h-3.5 w-3.5"/>
                        Save gates
                    </Button>
                    <Button size="sm" variant="ghost" className="h-7 gap-1.5" onClick={reset} disabled={isPending}>
                        <Undo2 className="h-3.5 w-3.5"/>
                        Reset
                    </Button>
                </div>
            )}
        </div>
    )
}
