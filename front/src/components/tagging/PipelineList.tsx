import {useState} from 'react'
import {toast} from 'sonner'
import type {DragEndEvent} from '@dnd-kit/core'
import {closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors,} from '@dnd-kit/core'
import {SortableContext, sortableKeyboardCoordinates, verticalListSortingStrategy,} from '@dnd-kit/sortable'
import {ServiceCard} from './ServiceCard'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {RuleServiceDetail, SegmentationServiceDetail} from '@/lib/types'

type PipelineService = RuleServiceDetail | SegmentationServiceDetail

interface PipelineListProps {
    services: PipelineService[]
}

export function PipelineList({services}: PipelineListProps) {
    const {reorder} = useTaggingMutations()

    // Keep only the ORDER locally (for smooth drag); the service objects are always
    // read fresh from props so enabled/requires/excludes edits reflect immediately.
    const [order, setOrder] = useState<string[]>(() => services.map((s) => s.id))

    const serverIds = services.map((s) => s.id)
    const sameSet = order.length === serverIds.length && order.every((id) => serverIds.includes(id))
    if (!sameSet) setOrder(serverIds)

    const byId = new Map(services.map((s) => [s.id, s]))
    const ordered = order.map((id) => byId.get(id)).filter((s): s is PipelineService => !!s)

    const sensors = useSensors(
        useSensor(PointerSensor),
        useSensor(KeyboardSensor, {coordinateGetter: sortableKeyboardCoordinates}),
    )

    const handleDragEnd = (event: DragEndEvent) => {
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
        return (
            <p className="rounded-lg border border-dashed border-border py-10 text-center text-sm text-muted-foreground">
                No rule or segmentation services yet. Create one with &ldquo;New service&rdquo;.
            </p>
        )
    }

    return (
        <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
            <SortableContext items={ordered.map((s) => s.id)} strategy={verticalListSortingStrategy}>
                <div className="space-y-2">
                    {ordered.map((service) => (
                        <ServiceCard key={service.id} service={service}/>
                    ))}
                </div>
            </SortableContext>
        </DndContext>
    )
}
