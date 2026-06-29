import {useState} from 'react'
import {Check, GripVertical, Pencil, Plus, Save, Trash2, Undo2} from 'lucide-react'
import {toast} from 'sonner'
import type {DragEndEvent} from '@dnd-kit/core'
import {closestCenter, DndContext, PointerSensor, useSensor, useSensors} from '@dnd-kit/core'
import {SortableContext, useSortable, verticalListSortingStrategy} from '@dnd-kit/sortable'
import {CSS} from '@dnd-kit/utilities'
import {Button} from '@/components/ui/button'
import {TagPicker} from '@/components/tags/TagPicker'
import {PredicateBuilder} from './PredicateBuilder'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {TagPath} from '@/lib/utils'
import {type BNode, describePredicate, deserialize, newRootNode, serialize} from '@/lib/predicate'
import {apiErrorMessage} from '@/api/client'
import type {RuleConfig, RuleTaggingRule} from '@/lib/types'

// Local draft rule — keeps a stable key so React/dnd can track unsaved rows.
interface DraftRule {
    key: string
    id?: string
    predicate: RuleTaggingRule['predicate']
    assign_tag: string
}

const nextKey = () => crypto.randomUUID()

interface RuleEditorProps {
    serviceId: string
    rules: RuleTaggingRule[]
}

/**
 * Edits the whole `rules[]` array as a local draft and commits via `PUT …/config`.
 * Array order = display/precedence order; reordering is just reordering then saving.
 */
export function RuleEditor({serviceId, rules}: RuleEditorProps) {
    const {replaceConfig} = useTaggingMutations()
    const [draft, setDraft] = useState<DraftRule[]>(() => rules.map((r) => ({key: nextKey(), ...r})))
    const [editingKey, setEditingKey] = useState<string | null>(null)

    // Resync from server when the persisted rules change (after save / external edit).
    const serverKey = JSON.stringify(rules.map((r) => [r.id, r.assign_tag, r.predicate]))
    const [syncedKey, setSyncedKey] = useState(serverKey)
    if (serverKey !== syncedKey) {
        setDraft(rules.map((r) => ({key: nextKey(), ...r})))
        setSyncedKey(serverKey)
        setEditingKey(null)
    }

    const dirty = JSON.stringify(draft.map((r) => [r.id, r.assign_tag, r.predicate])) !== serverKey

    const save = () => {
        const config: RuleConfig = {
            rules: draft.map((r) => ({...(r.id ? {id: r.id} : {}), predicate: r.predicate, assign_tag: r.assign_tag})),
        }
        replaceConfig.mutate(
            {id: serviceId, config},
            {onSuccess: () => setEditingKey(null), onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }
    const reset = () => {
        setDraft(rules.map((r) => ({key: nextKey(), ...r})))
        setEditingKey(null)
    }

    const sensors = useSensors(useSensor(PointerSensor, {activationConstraint: {distance: 4}}))
    const handleDragEnd = (e: DragEndEvent) => {
        const {active, over} = e
        if (!over || active.id === over.id) return
        const from = draft.findIndex((r) => r.key === active.id)
        const to = draft.findIndex((r) => r.key === over.id)
        if (from === -1 || to === -1) return
        const next = [...draft]
        const [moved] = next.splice(from, 1)
        next.splice(to, 0, moved)
        setDraft(next)
    }

    const addRule = () => {
        const key = nextKey()
        setDraft([...draft, {key, predicate: serialize(newRootNode()), assign_tag: ''}])
        setEditingKey(key)
    }

    return (
        <div className="space-y-3">
            {draft.length === 0 && <p className="text-sm text-muted-foreground">No rules yet.</p>}

            <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
                <SortableContext items={draft.map((r) => r.key)} strategy={verticalListSortingStrategy}>
                    <div className="space-y-1.5">
                        {draft.map((r) => (
                            <RuleRow
                                key={r.key}
                                rule={r}
                                editing={editingKey === r.key}
                                onEdit={() => setEditingKey(r.key)}
                                onClose={() => setEditingKey(null)}
                                onChange={(predicate, assign_tag) =>
                                    setDraft((d) => d.map((x) => (x.key === r.key ? {...x, predicate, assign_tag} : x)))
                                }
                                onDelete={() => setDraft((d) => d.filter((x) => x.key !== r.key))}
                            />
                        ))}
                    </div>
                </SortableContext>
            </DndContext>

            <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" className="h-7 gap-1.5 text-xs" onClick={addRule}>
                    <Plus className="h-3.5 w-3.5"/>
                    Add rule
                </Button>
                <div className="flex-1"/>
                {dirty && (
                    <>
                        <Button size="sm" className="h-7 gap-1.5" onClick={save} disabled={replaceConfig.isPending}>
                            <Save className="h-3.5 w-3.5"/>
                            Save rules
                        </Button>
                        <Button size="sm" variant="ghost" className="h-7 gap-1.5" onClick={reset} disabled={replaceConfig.isPending}>
                            <Undo2 className="h-3.5 w-3.5"/>
                            Reset
                        </Button>
                    </>
                )}
            </div>
        </div>
    )
}

function RuleRow({
                     rule,
                     editing,
                     onEdit,
                     onClose,
                     onChange,
                     onDelete,
                 }: {
    rule: DraftRule
    editing: boolean
    onEdit: () => void
    onClose: () => void
    onChange: (predicate: DraftRule['predicate'], assignTag: string) => void
    onDelete: () => void
}) {
    const {attributes, listeners, setNodeRef, transform, transition, isDragging} = useSortable({id: rule.key})
    const style = {transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.4 : 1}

    if (editing) {
        return (
            <div ref={setNodeRef} style={style}>
                <RuleEditForm rule={rule} onClose={onClose} onChange={onChange} onDelete={onDelete}/>
            </div>
        )
    }

    return (
        <div ref={setNodeRef} style={style} className="flex items-center gap-2 rounded-md border px-2 py-2 text-sm">
            <button className="cursor-grab touch-none text-muted-foreground/60 hover:text-foreground" {...attributes} {...listeners}
                    aria-label="Drag to reorder">
                <GripVertical className="h-3.5 w-3.5"/>
            </button>
            <code className="flex-1 font-mono text-xs">{describePredicate(rule.predicate)}</code>
            <span className="text-muted-foreground">→</span>
            <span className="flex-1">{rule.assign_tag ? TagPath.toDisplay(rule.assign_tag) : <em className="text-muted-foreground">no tag</em>}</span>
            <Button variant="ghost" size="sm" className="h-6 gap-1 px-2 text-xs text-muted-foreground hover:text-foreground" onClick={onEdit}>
                <Pencil className="h-3 w-3"/>
                Edit
            </Button>
            <Button variant="ghost" size="icon" className="h-6 w-6 text-muted-foreground hover:text-destructive" onClick={onDelete}
                    aria-label="Delete rule">
                <Trash2 className="h-3.5 w-3.5"/>
            </Button>
        </div>
    )
}

function RuleEditForm({
                          rule,
                          onClose,
                          onChange,
                          onDelete,
                      }: {
    rule: DraftRule
    onClose: () => void
    onChange: (predicate: DraftRule['predicate'], assignTag: string) => void
    onDelete: () => void
}) {
    const [root, setRoot] = useState<BNode>(() => deserialize(rule.predicate))
    const [assignTag, setAssignTag] = useState(rule.assign_tag)

    const apply = (next: BNode, tag: string) => {
        setRoot(next)
        setAssignTag(tag)
        onChange(serialize(next), tag)
    }

    return (
        <div className="space-y-3 rounded-md border border-primary/40 bg-primary/5 p-3">
            <PredicateBuilder value={root} onChange={(n) => apply(n, assignTag)}/>
            <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-muted-foreground">Assign tag</span>
                {assignTag && <span className="text-sm">{TagPath.toDisplay(assignTag)}</span>}
                <TagPicker onSelect={(t) => apply(root, t)} allowCreate triggerLabel={assignTag ? 'Change' : 'Pick tag'}/>
                <div className="flex-1"/>
                <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 gap-1 px-2 text-xs text-muted-foreground hover:text-destructive"
                    onClick={onDelete}
                >
                    <Trash2 className="h-3.5 w-3.5"/>
                    Discard
                </Button>
                <Button
                    size="sm"
                    className="h-7 gap-1 text-xs"
                    onClick={onClose}
                    disabled={!assignTag}
                    title={assignTag ? undefined : 'Pick a tag to assign first'}
                >
                    <Check className="h-3.5 w-3.5"/>
                    Done
                </Button>
            </div>
        </div>
    )
}
