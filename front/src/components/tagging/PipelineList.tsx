import {useState} from 'react'
import {GripVertical} from 'lucide-react'
import {toast} from 'sonner'
import type {DragEndEvent} from '@dnd-kit/core'
import {closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors} from '@dnd-kit/core'
import {SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy} from '@dnd-kit/sortable'
import {CSS} from '@dnd-kit/utilities'
import {ServiceRow} from './ServiceRow'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {RuleServiceDetail, SegmentationServiceDetail} from '@/lib/types'

type PipelineService = RuleServiceDetail | SegmentationServiceDetail

interface PipelineListProps {
    services: PipelineService[]
    selectedId: string | null
    onSelect: (id: string) => void
}

/** The reorderable rule + segmentation services (execution order = drag order). */
export function PipelineList({services, selectedId, onSelect}: PipelineListProps) {
    const {reorder, update, remove} = useTaggingMutations()

    // Keep only the ORDER locally for smooth drag; service objects are read fresh from props.
    const [order, setOrder] = useState<string[]>(() => services.map((s) => s.id))
    const serverIds = services.map((s) => s.id)
    const sameSet = order.length === serverIds.length && order.every((id) => serverIds.includes(id))
    if (!sameSet) setOrder(serverIds)

    const byId = new Map(services.map((s) => [s.id, s]))
    const ordered = order.map((id) => byId.get(id)).filter((s): s is PipelineService => !!s)

    const sensors = useSensors(useSensor(PointerSensor), useSensor(KeyboardSensor, {coordinateGetter: sortableKeyboardCoordinates}))

    const onDragEnd = (event: DragEndEvent) => {
        const {active, over} = event
        if (!over || active.id === over.id) return
        const oldIndex = order.indexOf(active.id as string)
        const newIndex = order.indexOf(over.id as string)
        if (oldIndex === -1 || newIndex === -1) return
        const next = [...order]
        const [moved] = next.splice(oldIndex, 1)
        next.splice(newIndex, 0, moved)
        setOrder(next)
        reorder.mutate(next, {onError: (err) => toast.error(apiErrorMessage(err))})
    }

    if (ordered.length === 0) {
        return <p className="px-1 py-6 text-center text-xs text-muted-foreground">No rule or segmentation services yet.</p>
    }

    return (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
            <SortableContext items={ordered.map((s) => s.id)} strategy={verticalListSortingStrategy}>
                <div className="space-y-1.5">
                    {ordered.map((service) => (
                        <SortableRow
                            key={service.id}
                            service={service}
                            selected={service.id === selectedId}
                            onSelect={() => onSelect(service.id)}
                            onToggle={(enabled) => update.mutate({
                                id: service.id,
                                body: {enabled}
                            }, {onError: (e) => toast.error(apiErrorMessage(e))})}
                            onDelete={(promoteTags) => remove.mutate({
                                id: service.id,
                                promoteTags
                            }, {onError: (e) => toast.error(apiErrorMessage(e))})}
                            togglePending={update.isPending}
                            deletePending={remove.isPending}
                        />
                    ))}
                </div>
            </SortableContext>
        </DndContext>
    )
}

function SortableRow({
                         service,
                         ...rest
                     }: {
    service: PipelineService
    selected: boolean
    onSelect: () => void
    onToggle: (enabled: boolean) => void
    onDelete: (promoteTags: boolean) => void
    togglePending: boolean
    deletePending: boolean
}) {
    const {attributes, listeners, setNodeRef, transform, transition, isDragging} = useSortable({id: service.id})
    const style = {transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.4 : 1}
    const handle = (
        <button
            className="cursor-grab touch-none text-muted-foreground/60 hover:text-foreground"
            onClick={(e) => e.stopPropagation()}
            {...attributes}
            {...listeners}
            aria-label="Drag to reorder"
        >
            <GripVertical className="h-3.5 w-3.5"/>
        </button>
    )
    return <ServiceRow service={service} dragHandle={handle} setNodeRef={setNodeRef} style={style} {...rest}/>
}
