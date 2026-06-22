import {useState} from 'react'
import {Check, Pencil, X} from 'lucide-react'
import {Input} from '@/components/ui/input'
import {Button} from '@/components/ui/button'

interface ServiceNameEditorProps {
    name: string
    /** Shown (muted) when the service has no name yet. */
    placeholder: string
    onRename: (name: string) => void
    isPending?: boolean
    className?: string
}

/** Inline display + edit of a tagging service's user-facing name. */
export function ServiceNameEditor({name, placeholder, onRename, isPending, className}: ServiceNameEditorProps) {
    const [editing, setEditing] = useState(false)
    const [draft, setDraft] = useState(name)

    if (editing) {
        const commit = () => {
            if (draft.trim() !== name) onRename(draft.trim())
            setEditing(false)
        }
        return (
            <span className={`flex items-center gap-1 ${className ?? ''}`}>
                <Input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                        if (e.key === 'Enter') commit()
                        if (e.key === 'Escape') setEditing(false)
                    }}
                    placeholder={placeholder}
                    maxLength={255}
                    className="h-7 w-48 text-sm"
                    disabled={isPending}
                />
                <Button variant="ghost" size="icon" className="h-6 w-6 text-emerald-500" onClick={commit} aria-label="Save name">
                    <Check className="h-3.5 w-3.5"/>
                </Button>
                <Button variant="ghost" size="icon" className="h-6 w-6 text-muted-foreground" onClick={() => setEditing(false)} aria-label="Cancel">
                    <X className="h-3.5 w-3.5"/>
                </Button>
            </span>
        )
    }

    return (
        <button
            type="button"
            onClick={() => {
                setDraft(name)
                setEditing(true)
            }}
            className={`group flex items-center gap-1.5 text-left ${className ?? ''}`}
            title="Rename"
        >
            <span className={name ? 'text-sm font-medium' : 'text-sm italic text-muted-foreground'}>
                {name || placeholder}
            </span>
            <Pencil className="h-3 w-3 text-muted-foreground/50 opacity-0 transition-opacity group-hover:opacity-100"/>
        </button>
    )
}
