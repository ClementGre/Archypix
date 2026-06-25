import {Link} from 'react-router-dom'
import {GripVertical, Pencil} from 'lucide-react'
import {toast} from 'sonner'
import {useSortable} from '@dnd-kit/sortable'
import {CSS} from '@dnd-kit/utilities'
import {Card, CardContent} from '@/components/ui/card'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Switch} from '@/components/ui/switch'
import {DeleteServiceDialog} from './DeleteServiceDialog'
import {ServiceNameEditor} from './ServiceNameEditor'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {RuleServiceDetail, SegmentationServiceDetail} from '@/lib/types'

type PipelineService = RuleServiceDetail | SegmentationServiceDetail

const TYPE_LABEL: Record<PipelineService['service_type'], string> = {
    rule: 'Rule',
    segmentation: 'Segmentation',
}

const TYPE_COLOR: Record<PipelineService['service_type'], string> = {
    rule: 'bg-violet-500/15 text-violet-500',
    segmentation: 'bg-sky-500/15 text-sky-500',
}

function itemCount(service: PipelineService): string {
    if (service.service_type === 'rule') {
        const n = service.rules.length
        return `${n} rule${n !== 1 ? 's' : ''}`
    }
    const n = service.segments.length
    return `${n} segment${n !== 1 ? 's' : ''}`
}

interface ServiceCardProps {
    service: PipelineService
}

export function ServiceCard({service}: ServiceCardProps) {
    const {update, remove} = useTaggingMutations()

    const {attributes, listeners, setNodeRef, transform, transition, isDragging} = useSortable({
        id: service.id,
    })

    const style = {
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.4 : 1,
    }

    const handleToggle = (enabled: boolean) => {
        update.mutate(
            {id: service.id, body: {enabled}},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    const handleDelete = (promoteTags: boolean) => {
        remove.mutate(
            {id: service.id, promoteTags},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    return (
        <div ref={setNodeRef} style={style}>
            <Card className="border">
                <CardContent className="p-4">
                    <div className="flex items-center gap-3">
                        {/* Drag handle */}
                        <button
                            className="cursor-grab touch-none text-muted-foreground hover:text-foreground"
                            {...attributes}
                            {...listeners}
                            aria-label="Drag to reorder"
                        >
                            <GripVertical className="h-4 w-4"/>
                        </button>

                        {/* Type badge */}
                        <Badge
                            variant="secondary"
                            className={`border-0 font-medium ${TYPE_COLOR[service.service_type]}`}
                        >
                            {TYPE_LABEL[service.service_type]}
                        </Badge>

                        {/* Name (inline editable) */}
                        <ServiceNameEditor
                            name={service.name}
                            placeholder={`Unnamed ${TYPE_LABEL[service.service_type].toLowerCase()}`}
                            onRename={(name) =>
                                update.mutate({id: service.id, body: {name}}, {onError: (err) => toast.error(apiErrorMessage(err))})
                            }
                            isPending={update.isPending}
                        />

                        {/* Item count */}
                        <span className="text-xs text-muted-foreground">{itemCount(service)}</span>

                        <div className="flex-1"/>

                        {/* Enabled switch */}
                        <div className="flex items-center gap-1.5">
                            <Switch
                                checked={service.enabled}
                                onCheckedChange={handleToggle}
                                disabled={update.isPending}
                                aria-label="Enable service"
                            />
                        </div>

                        {/* Edit — primary affordance (gates live on the editor page, not here,
                            so the list stays short when there are many services). */}
                        <Button variant="default" size="sm" asChild className="h-7 gap-1 text-xs">
                            <Link to={`/tagging/${service.id}`}>
                                <Pencil className="h-3 w-3"/>
                                Edit
                            </Link>
                        </Button>

                        {/* Delete */}
                        <DeleteServiceDialog onDelete={handleDelete} isPending={remove.isPending}/>
                    </div>
                </CardContent>
            </Card>
        </div>
    )
}
