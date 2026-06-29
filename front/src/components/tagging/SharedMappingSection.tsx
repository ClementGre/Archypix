import {toast} from 'sonner'
import {ServiceRow} from './ServiceRow'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {SharedTagMappingServiceDetail} from '@/lib/types'

interface SharedMappingSectionProps {
    services: SharedTagMappingServiceDetail[]
    selectedId: string | null
    onSelect: (id: string) => void
}

/** Shared-tag-mapping services — always run first, not reorderable (one per incoming share). */
export function SharedMappingSection({services, selectedId, onSelect}: SharedMappingSectionProps) {
    const {update, remove} = useTaggingMutations()

    if (services.length === 0) return null

    return (
        <div className="space-y-1.5">
            {services.map((service) => (
                <ServiceRow
                    key={service.id}
                    service={service}
                    selected={service.id === selectedId}
                    onSelect={() => onSelect(service.id)}
                    onToggle={(enabled) => update.mutate({id: service.id, body: {enabled}}, {onError: (e) => toast.error(apiErrorMessage(e))})}
                    onDelete={(promoteTags) => remove.mutate({id: service.id, promoteTags}, {onError: (e) => toast.error(apiErrorMessage(e))})}
                    togglePending={update.isPending}
                    deletePending={remove.isPending}
                />
            ))}
        </div>
    )
}
