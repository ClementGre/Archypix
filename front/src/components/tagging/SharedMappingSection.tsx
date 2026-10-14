import {ServiceRow} from './ServiceRow'
import type {SharedTagMappingServiceDetail} from '@/lib/types'

interface SharedMappingSectionProps {
    services: SharedTagMappingServiceDetail[]
    selectedId: string | null
    onSelect: (id: string) => void
}

/** Shared-tag-mapping services — always run first, not reorderable (one per incoming share). */
export function SharedMappingSection({services, selectedId, onSelect}: SharedMappingSectionProps) {
    if (services.length === 0) return null

    return (
        <div className="space-y-1.5">
            {services.map((service) => (
                <ServiceRow
                    key={service.id}
                    service={service}
                    selected={service.id === selectedId}
                    onSelect={() => onSelect(service.id)}
                />
            ))}
        </div>
    )
}
