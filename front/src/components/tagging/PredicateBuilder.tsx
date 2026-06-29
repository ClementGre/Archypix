// A nested block composer for the structured rule predicate tree (feature 13): AND / OR / NOT
// groups containing field-condition leaves and GPS-area leaves.
//
// One root @dnd-kit DndContext spans the whole tree, so a node can be dragged BETWEEN levels —
// out of an AND into its parent, into a sibling OR, etc. Each group is a SortableContext over its
// direct children plus a droppable "drop zone" (for appending / dropping into an empty group); on
// drop the dragged node is detached from its old parent and re-inserted at the target. A NOT holds
// a single child edited in place (not an independent drag target).

import {useState} from 'react'
import type {DragEndEvent, DragStartEvent} from '@dnd-kit/core'
import {closestCenter, DndContext, DragOverlay, PointerSensor, useDroppable, useSensor, useSensors} from '@dnd-kit/core'
import {SortableContext, useSortable, verticalListSortingStrategy} from '@dnd-kit/sortable'
import {CSS} from '@dnd-kit/utilities'
import {ChevronsUpDown, GripVertical, MapPin, Plus, Trash2} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {NumberInput} from '@/components/ui/number-input'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {Checkbox} from '@/components/ui/checkbox'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList} from '@/components/ui/command'
import {DateRangePicker} from '@/components/common/DateRangePicker'
import {MapZonePopover, type Zone} from './MapZonePopover'
import {
    type BNode,
    type CondState,
    detach,
    FIELD_GROUPS,
    fieldDef,
    fieldsByGroup,
    type FieldType,
    IGNORE_CASE_OPS,
    insertInto,
    isWithin,
    locate,
    newFieldNode,
    newGpsBboxNode,
    newGpsRadiusNode,
    newGroupNode,
    operatorsFor,
    SEASONS,
} from '@/lib/predicate'

interface NodeViewProps {
    node: BNode
    onChange: (next: BNode) => void
    /** Remove this node from its parent group. Absent for the root and for a NOT's child. */
    onRemove?: () => void
    isRoot?: boolean
}

const CONTAINER_PREFIX = 'container:'

export function PredicateBuilder({value, onChange}: { value: BNode; onChange: (n: BNode) => void }) {
    const [activeId, setActiveId] = useState<string | null>(null)
    const sensors = useSensors(useSensor(PointerSensor, {activationConstraint: {distance: 4}}))

    const handleDragStart = (e: DragStartEvent) => setActiveId(String(e.active.id))

    const handleDragEnd = (e: DragEndEvent) => {
        setActiveId(null)
        const {active, over} = e
        if (!over) return
        const activeKey = String(active.id)
        const overId = String(over.id)

        let containerId: string
        let index: number
        if (overId.startsWith(CONTAINER_PREFIX)) {
            containerId = overId.slice(CONTAINER_PREFIX.length)
            index = Number.MAX_SAFE_INTEGER // append (insertInto clamps)
        } else {
            if (overId === activeKey) return
            const loc = locate(value, overId)
            if (!loc) return
            containerId = loc.containerId
            index = loc.index
        }

        // Don't drop a node into itself or its own subtree.
        if (containerId === activeKey || isWithin(value, activeKey, containerId)) return

        const src = locate(value, activeKey)
        if (!src) return
        if (src.containerId === containerId && src.index === index) return

        const {tree, node} = detach(value, activeKey)
        if (!node) return
        // Removing an earlier sibling in the same container shifts the target index left.
        let idx = index
        if (src.containerId === containerId && src.index < index && index !== Number.MAX_SAFE_INTEGER) {
            idx -= 1
        }
        onChange(insertInto(tree, containerId, idx, node))
    }

    return (
        <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragStart={handleDragStart}
            onDragEnd={handleDragEnd}
            onDragCancel={() => setActiveId(null)}
        >
            <NodeView node={value} onChange={onChange} isRoot/>
            <DragOverlay>
                {activeId ? (
                    <div className="rounded-md border bg-popover px-2 py-1 text-xs shadow-md">Moving block…</div>
                ) : null}
            </DragOverlay>
        </DndContext>
    )
}

function NodeView(props: NodeViewProps) {
    switch (props.node.kind) {
        case 'group':
        case 'not':
            return <LogicalView {...props}/>
        case 'field':
            return <FieldView {...props}/>
        case 'gps_bbox':
        case 'gps_radius':
            return <GpsView {...props}/>
    }
}

// ── Logical node (AND / OR / NOT) ────────────────────────────────────────────────

const OP_STYLES: Record<string, string> = {
    and: 'border-l-emerald-500/60',
    or: 'border-l-sky-500/60',
    not: 'border-l-rose-500/60',
}

function currentOp(node: BNode): 'and' | 'or' | 'not' {
    if (node.kind === 'not') return 'not'
    if (node.kind === 'group') return node.op
    return 'and'
}

function LogicalView({node, onChange, onRemove, isRoot}: NodeViewProps) {
    const op = currentOp(node)

    const setOp = (next: 'and' | 'or' | 'not') => {
        if (next === op) return
        if (next === 'not') {
            const kids = node.kind === 'group' ? node.children : [node]
            const child: BNode =
                kids.length === 1 ? kids[0] : {...newGroupNode('and'), children: kids}
            onChange({id: node.id, kind: 'not', child})
        } else if (node.kind === 'not') {
            onChange({id: node.id, kind: 'group', op: next, children: [node.child]})
        } else if (node.kind === 'group') {
            onChange({...node, op: next})
        }
    }

    const replaceChild = (idx: number, next: BNode) => {
        if (node.kind === 'not') {
            onChange({...node, child: next})
        } else if (node.kind === 'group') {
            const copy = [...node.children]
            copy[idx] = next
            onChange({...node, children: copy})
        }
    }

    const removeChild = (idx: number) => {
        if (node.kind === 'group') {
            onChange({...node, children: node.children.filter((_, i) => i !== idx)})
        }
    }

    const addChild = (child: BNode) => {
        if (node.kind === 'group') onChange({...node, children: [...node.children, child]})
    }

    return (
        <div className={`rounded-md border border-l-4 bg-muted/20 p-2.5 ${OP_STYLES[op]}`}>
            <div className="mb-2 flex items-center gap-2">
                <div className="inline-flex overflow-hidden rounded-md border text-xs">
                    {(['and', 'or', 'not'] as const).map((o) => (
                        <button
                            key={o}
                            type="button"
                            onClick={() => setOp(o)}
                            className={`px-2 py-0.5 font-medium uppercase transition-colors ${
                                op === o ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-accent'
                            }`}
                        >
                            {o}
                        </button>
                    ))}
                </div>
                <span className="text-xs text-muted-foreground">
                    {op === 'not' ? 'inverts the block below' : op === 'and' ? 'all must match' : 'any must match'}
                </span>
                <div className="flex-1"/>
                {!isRoot && onRemove && (
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-6 w-6 text-muted-foreground hover:text-destructive"
                        onClick={onRemove}
                        aria-label="Remove block"
                    >
                        <Trash2 className="h-3.5 w-3.5"/>
                    </Button>
                )}
            </div>

            {node.kind === 'not' ? (
                <div className="pl-1">
                    <NodeView node={node.child} onChange={(c) => replaceChild(0, c)}/>
                </div>
            ) : node.kind === 'group' ? (
                <GroupChildren
                    node={node}
                    onReplaceChild={replaceChild}
                    onRemoveChild={removeChild}
                />
            ) : null}

            {node.kind === 'group' && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                    <AddButton label="Condition" onClick={() => addChild(newFieldNode())}/>
                    <AddButton label="AND group" onClick={() => addChild(newGroupNode('and'))}/>
                    <AddButton label="OR group" onClick={() => addChild(newGroupNode('or'))}/>
                    <AddButton label="GPS box" icon onClick={() => addChild(newGpsBboxNode())}/>
                    <AddButton label="GPS radius" icon onClick={() => addChild(newGpsRadiusNode())}/>
                </div>
            )}
        </div>
    )
}

function GroupChildren({
                           node,
                           onReplaceChild,
                           onRemoveChild,
                       }: {
    node: Extract<BNode, { kind: 'group' }>
    onReplaceChild: (idx: number, next: BNode) => void
    onRemoveChild: (idx: number) => void
}) {
    const {setNodeRef, isOver} = useDroppable({id: CONTAINER_PREFIX + node.id})
    return (
        <SortableContext items={node.children.map((c) => c.id)} strategy={verticalListSortingStrategy}>
            <div
                ref={setNodeRef}
                className={`min-h-[1.5rem] space-y-1.5 rounded-md ${isOver ? 'bg-primary/5 outline-dashed outline-1 outline-primary/30' : ''}`}
            >
                {node.children.length === 0 && (
                    <p className="px-1 py-1 text-xs text-muted-foreground">Empty block — add or drop a condition here.</p>
                )}
                {node.children.map((child, idx) => (
                    <SortableRow key={child.id} id={child.id}>
                        <NodeView
                            node={child}
                            onChange={(c) => onReplaceChild(idx, c)}
                            onRemove={() => onRemoveChild(idx)}
                        />
                    </SortableRow>
                ))}
            </div>
        </SortableContext>
    )
}

function AddButton({label, onClick, icon}: { label: string; onClick: () => void; icon?: boolean }) {
    return (
        <Button variant="outline" size="sm" className="h-6 gap-1 px-2 text-xs" onClick={onClick}>
            {icon ? <MapPin className="h-3 w-3"/> : <Plus className="h-3 w-3"/>}
            {label}
        </Button>
    )
}

/** Sortable wrapper giving each group child a drag handle (used for both reorder and reparent). */
function SortableRow({id, children}: { id: string; children: React.ReactNode }) {
    const {attributes, listeners, setNodeRef, transform, transition, isDragging} = useSortable({id})
    const style = {transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.4 : 1}
    return (
        <div ref={setNodeRef} style={style} className="flex items-start gap-1">
            <button
                className="mt-1.5 cursor-grab touch-none text-muted-foreground/60 hover:text-foreground"
                {...attributes}
                {...listeners}
                aria-label="Drag to move or reorder"
            >
                <GripVertical className="h-3.5 w-3.5"/>
            </button>
            <div className="flex-1">{children}</div>
        </div>
    )
}

// ── Field leaf ───────────────────────────────────────────────────────────────────

/** A fresh condition state when the operator changes. */
function condForOp(type: FieldType, op: string): CondState {
    // is_present / is_absent take no operand — the operator itself is the whole condition.
    if (op === 'is_present' || op === 'is_absent') return {op}
    if (op === 'between') return {op, value: '', value2: ''}
    if (op === 'date_range') return {op, from: '', to: ''}
    if (op === 'time_range') return {op, from: '', to: ''}
    if (op === 'season') return {op, value: 'summer'}
    if (op === 'month') return {op, value: '1'}
    if (type === 'bool') return {op: 'eq', value: 'true'}
    return {op, value: ''}
}

/** A searchable, type-grouped field selector (Dates / Camera / Location / File / Ownership). */
function FieldPicker({value, onChange}: { value: string; onChange: (field: string) => void }) {
    const [open, setOpen] = useState(false)
    const [query, setQuery] = useState('')
    const current = fieldDef(value)
    const q = query.trim().toLowerCase()

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>
                <Button variant="outline" size="sm" className="h-7 w-[9.5rem] justify-between gap-1 px-2 text-xs font-normal">
                    <span className="truncate">{current?.label ?? 'Field'}</span>
                    <ChevronsUpDown className="h-3 w-3 opacity-50"/>
                </Button>
            </PopoverTrigger>
            <PopoverContent className="w-56 p-0" align="start">
                <Command shouldFilter={false}>
                    <CommandInput value={query} onValueChange={setQuery} placeholder="Search field…"/>
                    <CommandList>
                        <CommandEmpty>No field found.</CommandEmpty>
                        {FIELD_GROUPS.map((group) => {
                            const fields = fieldsByGroup(group).filter((f) => f.label.toLowerCase().includes(q))
                            if (fields.length === 0) return null
                            return (
                                <CommandGroup key={group} heading={group}>
                                    {fields.map((f) => (
                                        <CommandItem
                                            key={f.name}
                                            value={f.name}
                                            onSelect={() => {
                                                onChange(f.name)
                                                setOpen(false)
                                                setQuery('')
                                            }}
                                            className="text-xs"
                                        >
                                            {f.label}
                                        </CommandItem>
                                    ))}
                                </CommandGroup>
                            )
                        })}
                    </CommandList>
                </Command>
            </PopoverContent>
        </Popover>
    )
}

function FieldView({node, onChange, onRemove}: NodeViewProps) {
    if (node.kind !== 'field') return null
    const def = fieldDef(node.field)!
    const ops = operatorsFor(def.type)
    const cond = node.cond

    const setField = (field: string) => {
        const nd = fieldDef(field)!
        onChange({...node, field, cond: condForOp(nd.type, operatorsFor(nd.type)[0].op)})
    }
    const setOp = (op: string) => onChange({...node, cond: condForOp(def.type, op)})
    const setCond = (patch: Partial<CondState>) => onChange({...node, cond: {...cond, ...patch}})

    return (
        <div className="flex flex-wrap items-center gap-1.5 rounded-md border bg-background px-2 py-1.5">
            <FieldPicker value={node.field} onChange={setField}/>

            <Select value={cond.op} onValueChange={setOp}>
                <SelectTrigger className="h-7 w-[7.5rem] text-xs">
                    <SelectValue/>
                </SelectTrigger>
                <SelectContent>
                    {ops.map((o) => (
                        <SelectItem key={o.op} value={o.op} className="text-xs">
                            {o.label}
                        </SelectItem>
                    ))}
                </SelectContent>
            </Select>

            <CondValue type={def.type} cond={cond} unit={def.unit} onChange={setCond}/>

            {/* Case-insensitivity for string comparisons (feature 15) — not presence checks. */}
            {def.type === 'str' && IGNORE_CASE_OPS.has(cond.op) && (
                <label className="flex select-none items-center gap-1 text-xs text-muted-foreground">
                    <Checkbox
                        checked={!!cond.ignoreCase}
                        onCheckedChange={(v) => setCond({ignoreCase: v === true})}
                        className="h-3.5 w-3.5"
                    />
                    ignore case
                </label>
            )}

            <div className="flex-1"/>
            {onRemove && (
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 text-muted-foreground hover:text-destructive"
                    onClick={onRemove}
                    aria-label="Remove condition"
                >
                    <Trash2 className="h-3.5 w-3.5"/>
                </Button>
            )}
        </div>
    )
}

const NUM = (type: FieldType) => (type === 'float' ? 'any' : '1')

function CondValue({
                       type,
                       cond,
                       unit,
                       onChange,
                   }: {
    type: FieldType
    cond: CondState
    unit?: string
    onChange: (patch: Partial<CondState>) => void
}) {
    const numStep = NUM(type)
    const unitTag = unit && <span className="text-xs text-muted-foreground">{unit}</span>

    switch (cond.op) {
        // is_present / is_absent are self-contained operators (no value field).
        case 'is_present':
        case 'is_absent':
            return null
        case 'eq':
            if (type === 'bool') {
                return (
                    <Select value={cond.value ?? 'true'} onValueChange={(v) => onChange({value: v})}>
                        <SelectTrigger className="h-7 w-20 text-xs"><SelectValue/></SelectTrigger>
                        <SelectContent>
                            <SelectItem value="true" className="text-xs">yes</SelectItem>
                            <SelectItem value="false" className="text-xs">no</SelectItem>
                        </SelectContent>
                    </Select>
                )
            }
            if (type === 'str') {
                return <TextVal value={cond.value} onChange={(v) => onChange({value: v})}/>
            }
            return (
                <span className="flex items-center gap-1">
                    <NumVal step={numStep} value={cond.value} onChange={(v) => onChange({value: v})}/>
                    {unitTag}
                </span>
            )
        case 'min':
        case 'max':
            return (
                <span className="flex items-center gap-1">
                    <NumVal step={numStep} value={cond.value} onChange={(v) => onChange({value: v})}/>
                    {unitTag}
                </span>
            )
        case 'between':
            return (
                <span className="flex items-center gap-1">
                    <NumVal step={numStep} value={cond.value} onChange={(v) => onChange({value: v})}/>
                    <span className="text-xs text-muted-foreground">to</span>
                    <NumVal step={numStep} value={cond.value2} onChange={(v) => onChange({value2: v})}/>
                    {unitTag}
                </span>
            )
        case 'eq_ic':
        case 'contains':
        case 'starts_with':
        case 'ends_with':
        case 'regex':
            return <TextVal value={cond.value} onChange={(v) => onChange({value: v})}/>
        case 'year':
            return <NumVal step="1" value={cond.value} onChange={(v) => onChange({value: v})} placeholder="2024" width="w-28"/>
        case 'month':
            return (
                <Select value={cond.value ?? '1'} onValueChange={(v) => onChange({value: v})}>
                    <SelectTrigger className="h-7 w-24 text-xs"><SelectValue/></SelectTrigger>
                    <SelectContent>
                        {Array.from({length: 12}, (_, i) => String(i + 1)).map((m) => (
                            <SelectItem key={m} value={m} className="text-xs">{m}</SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            )
        case 'season':
            return (
                <Select value={cond.value ?? 'summer'} onValueChange={(v) => onChange({value: v})}>
                    <SelectTrigger className="h-7 w-28 text-xs"><SelectValue/></SelectTrigger>
                    <SelectContent>
                        {SEASONS.map((s) => (
                            <SelectItem key={s} value={s} className="text-xs capitalize">{s}</SelectItem>
                        ))}
                    </SelectContent>
                </Select>
            )
        case 'date_range':
            return (
                <DateRangePicker
                    mode="datetime"
                    from={cond.from ?? ''}
                    to={cond.to ?? ''}
                    onChange={(f, t) => onChange({from: f, to: t})}
                />
            )
        case 'time_range':
            return (
                <span className="flex items-center gap-1">
                    <Input type="time" value={cond.from ?? ''} onChange={(e) => onChange({from: e.target.value})} className="h-7 w-24 text-xs"/>
                    <span className="text-xs text-muted-foreground">→</span>
                    <Input type="time" value={cond.to ?? ''} onChange={(e) => onChange({to: e.target.value})} className="h-7 w-24 text-xs"/>
                </span>
            )
        default:
            return null
    }
}

function NumVal({
                    value,
                    onChange,
                    step,
                    placeholder,
                    width = 'w-28',
                }: {
    value: string | undefined
    onChange: (v: string) => void
    step: string
    placeholder?: string
    width?: string
}) {
    return (
        <NumberInput
            step={step}
            value={value ?? ''}
            placeholder={placeholder}
            onChange={(e) => onChange(e.target.value)}
            className={`h-7 ${width} text-xs`}
        />
    )
}

function TextVal({value, onChange}: { value: string | undefined; onChange: (v: string) => void }) {
    return (
        <Input
            value={value ?? ''}
            onChange={(e) => onChange(e.target.value)}
            className="h-7 w-40 text-xs"
            placeholder="value"
        />
    )
}

// ── GPS leaves ───────────────────────────────────────────────────────────────────

function GpsView({node, onChange, onRemove}: NodeViewProps) {
    if (node.kind !== 'gps_bbox' && node.kind !== 'gps_radius') return null

    const zone: Zone =
        node.kind === 'gps_bbox' ? {kind: 'bbox', box: node.box} : {kind: 'circle', radius: node.radius}

    const onZone = (z: Zone) =>
        onChange(
            z.kind === 'bbox'
                ? {id: node.id, kind: 'gps_bbox', box: z.box}
                : {id: node.id, kind: 'gps_radius', radius: z.radius},
        )

    return (
        <div className="flex flex-wrap items-center gap-2 rounded-md border bg-background px-2 py-1.5">
            <span className="flex items-center gap-1 text-xs font-medium">
                <MapPin className="h-3.5 w-3.5 text-muted-foreground"/>
                GPS area
            </span>
            <MapZonePopover zone={zone} onChange={onZone}/>
            <div className="flex-1"/>
            {onRemove && (
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 text-muted-foreground hover:text-destructive"
                    onClick={onRemove}
                    aria-label="Remove GPS area"
                >
                    <Trash2 className="h-3.5 w-3.5"/>
                </Button>
            )}
        </div>
    )
}
