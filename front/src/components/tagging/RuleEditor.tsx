import {useState} from 'react'
import {Check, GripVertical, Pencil, Trash2, X} from 'lucide-react'
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
import type {RuleTaggingRule} from '@/lib/types'

interface RuleEditorProps {
    serviceId: string
    rules: RuleTaggingRule[]
}

export function RuleEditor({serviceId, rules}: RuleEditorProps) {
    const {reorderRules} = useTaggingMutations()
    const [editingId, setEditingId] = useState<string | null>(null)

    // Keep only the ORDER locally for smooth drag; rule objects are read fresh from props.
    const [order, setOrder] = useState<string[]>(() => rules.map((r) => r.id))
    const serverIds = rules.map((r) => r.id)
    const sameSet = order.length === serverIds.length && order.every((id) => serverIds.includes(id))
    if (!sameSet) setOrder(serverIds)
    const byId = new Map(rules.map((r) => [r.id, r]))
    const ordered = order.map((id) => byId.get(id)).filter((r): r is RuleTaggingRule => !!r)

    const sensors = useSensors(useSensor(PointerSensor, {activationConstraint: {distance: 4}}))

    const handleDragEnd = (e: DragEndEvent) => {
        const {active, over} = e
        if (!over || active.id === over.id) return
        const from = order.indexOf(active.id as string)
        const to = order.indexOf(over.id as string)
        if (from === -1 || to === -1) return
        const next = [...order]
        const [moved] = next.splice(from, 1)
        next.splice(to, 0, moved)
        setOrder(next)
        reorderRules.mutate({serviceId, orderedIds: next}, {onError: (err) => toast.error(apiErrorMessage(err))})
    }

    return (
        <div className="space-y-3">
            {rules.length === 0 && <p className="text-sm text-muted-foreground">No rules yet.</p>}

            <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
                <SortableContext items={ordered.map((r) => r.id)} strategy={verticalListSortingStrategy}>
                    <div className="space-y-1.5">
                        {ordered.map((r) => (
                            <RuleRow
                                key={r.id}
                                serviceId={serviceId}
                                rule={r}
                                editing={editingId === r.id}
                                onEdit={() => setEditingId(r.id)}
                                onClose={() => setEditingId(null)}
                            />
                        ))}
                    </div>
                </SortableContext>
            </DndContext>

            <AddRuleForm serviceId={serviceId}/>
        </div>
    )
}

// ── A single rule row: read-only summary, or an inline editor ────────────────────

function RuleRow({
                     serviceId,
                     rule,
                     editing,
                     onEdit,
                     onClose,
                 }: {
    serviceId: string
    rule: RuleTaggingRule
    editing: boolean
    onEdit: () => void
    onClose: () => void
}) {
    const {editRule, deleteRule} = useTaggingMutations()
    const {attributes, listeners, setNodeRef, transform, transition, isDragging} = useSortable({id: rule.id})
    const style = {transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.4 : 1}

    if (editing) {
        return (
            <div ref={setNodeRef} style={style}>
                <RuleEditForm
                    rule={rule}
                    onCancel={onClose}
                    onSave={(predicate, assign_tag) =>
                        editRule.mutate(
                            {serviceId, ruleId: rule.id, predicate, assign_tag},
                            {onSuccess: onClose, onError: (err) => toast.error(apiErrorMessage(err))},
                        )
                    }
                    saving={editRule.isPending}
                />
            </div>
        )
    }

    return (
        <div ref={setNodeRef} style={style} className="flex items-center gap-2 rounded-md border px-2 py-2 text-sm">
            <button
                className="cursor-grab touch-none text-muted-foreground/60 hover:text-foreground"
                {...attributes}
                {...listeners}
                aria-label="Drag to reorder"
            >
                <GripVertical className="h-3.5 w-3.5"/>
            </button>
            <code className="flex-1 font-mono text-xs">{describePredicate(rule.predicate)}</code>
            <span className="text-muted-foreground">→</span>
            <span className="flex-1">{TagPath.toDisplay(rule.assign_tag)}</span>
            <Button variant="ghost" size="icon" className="h-6 w-6 text-muted-foreground hover:text-foreground" onClick={onEdit}
                    aria-label="Edit rule">
                <Pencil className="h-3.5 w-3.5"/>
            </Button>
            <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 text-muted-foreground hover:text-destructive"
                onClick={() => deleteRule.mutate({serviceId, ruleId: rule.id}, {onError: (err) => toast.error(apiErrorMessage(err))})}
                disabled={deleteRule.isPending}
                aria-label="Delete rule"
            >
                <Trash2 className="h-3.5 w-3.5"/>
            </Button>
        </div>
    )
}

function RuleEditForm({
                          rule,
                          onCancel,
                          onSave,
                          saving,
                      }: {
    rule: RuleTaggingRule
    onCancel: () => void
    onSave: (predicate: ReturnType<typeof serialize>, assignTag: string) => void
    saving: boolean
}) {
    const [root, setRoot] = useState<BNode>(() => deserialize(rule.predicate))
    const [assignTag, setAssignTag] = useState(rule.assign_tag)

    return (
        <div className="space-y-3 rounded-md border border-primary/40 bg-primary/5 p-3">
            <PredicateBuilder value={root} onChange={setRoot}/>
            <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-muted-foreground">Assign tag</span>
                {assignTag && <span className="text-sm">{TagPath.toDisplay(assignTag)}</span>}
                <TagPicker onSelect={setAssignTag} allowCreate={true} triggerLabel={assignTag ? 'Change' : 'Pick tag'}/>
                <div className="flex-1"/>
                <Button variant="ghost" size="sm" className="h-7 gap-1 text-xs" onClick={onCancel}>
                    <X className="h-3.5 w-3.5"/>
                    Cancel
                </Button>
                <Button size="sm" className="h-7 gap-1 text-xs" disabled={!assignTag || saving} onClick={() => onSave(serialize(root), assignTag)}>
                    <Check className="h-3.5 w-3.5"/>
                    Save
                </Button>
            </div>
        </div>
    )
}

// ── Add-rule composer ────────────────────────────────────────────────────────────

function AddRuleForm({serviceId}: { serviceId: string }) {
    const {addRule} = useTaggingMutations()
    const [root, setRoot] = useState<BNode>(() => newRootNode())
    const [assignTag, setAssignTag] = useState('')

    const handleAdd = () => {
        if (!assignTag) return
        addRule.mutate(
            {serviceId, predicate: serialize(root), assign_tag: assignTag},
            {
                onSuccess: () => {
                    setRoot(newRootNode())
                    setAssignTag('')
                },
                onError: (err) => toast.error(apiErrorMessage(err)),
            },
        )
    }

    return (
        <div className="rounded-md border border-dashed p-3 space-y-3">
            <div>
                <label className="mb-1.5 block text-xs text-muted-foreground">New rule condition</label>
                <PredicateBuilder value={root} onChange={setRoot}/>
                <p className="mt-1.5 text-xs text-muted-foreground">
                    Compose blocks of AND / OR / NOT — drag the handle to move a block between levels. An empty AND
                    block matches every picture.
                </p>
            </div>
            <div className="flex flex-wrap items-end gap-2">
                <div>
                    <label className="mb-1 block text-xs text-muted-foreground">Assign tag</label>
                    <div className="flex items-center gap-1.5">
                        {assignTag && <span className="text-sm">{TagPath.toDisplay(assignTag)}</span>}
                        <TagPicker onSelect={setAssignTag} allowCreate={true} triggerLabel={assignTag ? 'Change' : 'Pick tag'}/>
                    </div>
                </div>
                <Button size="sm" onClick={handleAdd} disabled={!assignTag || addRule.isPending}>
                    Add rule
                </Button>
            </div>
        </div>
    )
}
