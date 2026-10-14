import type {ServiceType} from '@/lib/types'

export const SERVICE_TYPE_LABEL: Record<ServiceType, string> = {
    rule: 'Rule',
    segmentation: 'Segmentation',
    shared_tag_mapping: 'Mapping',
}

export const SERVICE_TYPE_COLOR: Record<ServiceType, string> = {
    rule: 'bg-violet-500/15 text-violet-500',
    segmentation: 'bg-sky-500/15 text-sky-500',
    shared_tag_mapping: 'bg-amber-500/15 text-amber-500',
}
