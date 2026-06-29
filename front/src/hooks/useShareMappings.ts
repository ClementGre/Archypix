import {useMemo} from 'react'
import {useTaggingMutations, useTaggingServices} from '@/hooks/useTaggingServices'
import type {SharedTagMappingServiceDetail} from '@/lib/types'

export interface ShareMapping {
    serviceId: string
    assign_tag: string
    is_broken: boolean
}

/**
 * Helper around the shared-tag-mapping services for wiring an incoming share to local tags.
 * Feature 20: one service per share, its config a single `incoming_share_id` + `assign_tags`.
 * A share may map to SEVERAL local tags — adding appends, removing drops a single tag (deleting
 * the service once its last tag is removed).
 */
export function useShareMappings() {
    const {data: services} = useTaggingServices()
    const {create, replaceConfig, remove} = useTaggingMutations()

    const mappingServices = useMemo(
        () => (services ?? []).filter((s): s is SharedTagMappingServiceDetail => s.service_type === 'shared_tag_mapping'),
        [services],
    )

    const serviceFor = (incomingShareId: string) => mappingServices.find((s) => s.incoming_share_id === incomingShareId)

    const forShare = (incomingShareId: string): ShareMapping[] => {
        const svc = serviceFor(incomingShareId)
        if (!svc) return []
        return svc.assign_tags.map((tag) => ({serviceId: svc.id, assign_tag: tag, is_broken: svc.is_broken}))
    }

    const addMapping = async (incomingShareId: string, name: string | undefined, assignTag: string) => {
        const svc = serviceFor(incomingShareId)
        if (svc) {
            if (svc.assign_tags.includes(assignTag)) return
            const config = {incoming_share_id: incomingShareId, assign_tags: [...svc.assign_tags, assignTag]}
            await replaceConfig.mutateAsync({id: svc.id, config})
        } else {
            await create.mutateAsync({
                service_type: 'shared_tag_mapping',
                name,
                config: {incoming_share_id: incomingShareId, assign_tags: [assignTag]}
            })
        }
    }

    const removeTag = async (incomingShareId: string, assignTag: string) => {
        const svc = serviceFor(incomingShareId)
        if (!svc) return
        const remaining = svc.assign_tags.filter((t) => t !== assignTag)
        if (remaining.length === 0) {
            await remove.mutateAsync({id: svc.id, promoteTags: false})
        } else {
            await replaceConfig.mutateAsync({id: svc.id, config: {incoming_share_id: incomingShareId, assign_tags: remaining}})
        }
    }

    const isBusy = create.isPending || replaceConfig.isPending || remove.isPending

    return {mappingServices, forShare, addMapping, removeTag, isBusy}
}
