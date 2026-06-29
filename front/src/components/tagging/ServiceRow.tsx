import type {CSSProperties, ReactNode} from 'react'
import {AlertTriangle} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Switch} from '@/components/ui/switch'
import {DeleteServiceDialog} from './DeleteServiceDialog'
import type {ServiceDetailResponse, ServiceType} from '@/lib/types'

const TYPE_LABEL: Record<ServiceType, string> = {
    rule: 'Rule',
    segmentation: 'Segmentation',
    shared_tag_mapping: 'Mapping',
}

const TYPE_COLOR: Record<ServiceType, string> = {
    rule: 'bg-violet-500/15 text-violet-500',
    segmentation: 'bg-sky-500/15 text-sky-500',
    shared_tag_mapping: 'bg-amber-500/15 text-amber-500',
}

export function serviceTypeLabel(t: ServiceType): string {
    return TYPE_LABEL[t]
}

function itemCount(service: ServiceDetailResponse): string {
    switch (service.service_type) {
        case 'rule': {
            const n = service.rules.length
            return `${n} rule${n !== 1 ? 's' : ''}`
        }
        case 'segmentation': {
            const n = service.config.bands.length
            return `${n} band${n !== 1 ? 's' : ''}`
        }
        case 'shared_tag_mapping': {
            const n = service.assign_tags.length
            return `${n} tag${n !== 1 ? 's' : ''}`
        }
    }
}

interface ServiceRowProps {
    service: ServiceDetailResponse
    selected: boolean
    onSelect: () => void
    onToggle: (enabled: boolean) => void
    onDelete: (promoteTags: boolean) => void
    togglePending: boolean
    deletePending: boolean
    /** Drag-handle node (pipeline rows only); omitted for non-reorderable mappings. */
    dragHandle?: ReactNode
    setNodeRef?: (el: HTMLElement | null) => void
    style?: CSSProperties
}

/** A compact, selectable row in the left-pane service list. */
export function ServiceRow({
                               service,
                               selected,
                               onSelect,
                               onToggle,
                               onDelete,
                               togglePending,
                               deletePending,
                               dragHandle,
                               setNodeRef,
                               style
                           }: ServiceRowProps) {
    const broken = service.service_type === 'shared_tag_mapping' && service.is_broken
    const stop = (e: { stopPropagation: () => void }) => e.stopPropagation()

    return (
        <div
            ref={setNodeRef}
            style={style}
            onClick={onSelect}
            className={`flex cursor-pointer items-center gap-2 rounded-md border px-2 py-2 text-sm transition-colors ${
                selected ? 'border-primary bg-primary/5' : 'hover:bg-muted/50'
            }`}
        >
            {dragHandle}
            <Badge variant="secondary" className={`shrink-0 border-0 text-[11px] font-medium ${TYPE_COLOR[service.service_type]}`}>
                {TYPE_LABEL[service.service_type]}
            </Badge>
            <div className="min-w-0 flex-1">
                <div className={`truncate ${service.name ? '' : 'italic text-muted-foreground'}`}>
                    {service.name || `Unnamed ${TYPE_LABEL[service.service_type].toLowerCase()}`}
                </div>
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                    {itemCount(service)}
                    {broken && (
                        <span className="inline-flex items-center gap-0.5 text-red-500">
                            <AlertTriangle className="h-3 w-3"/> broken
                        </span>
                    )}
                </div>
            </div>
            <span onClick={stop} className="flex items-center gap-1">
                <Switch checked={service.enabled} onCheckedChange={onToggle} disabled={togglePending} aria-label="Enable service"/>
                <DeleteServiceDialog onDelete={onDelete} isPending={deletePending}/>
            </span>
        </div>
    )
}
