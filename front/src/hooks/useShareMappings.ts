import {useMemo} from 'react'
import {useTaggingMutations, useTaggingServices} from '@/hooks/useTaggingServices'
import type {SharedTagMappingServiceDetail} from '@/lib/types'

export interface ShareMapping {
    serviceId: string
    ruleId: string
    assign_tag: string
    is_broken: boolean
}

/**
 * Helper around the SharedTagMappingService(s) for wiring incoming shares to a
 * local tag. A share may have a SINGLE mapping, so adding replaces any existing
 * one. Adding the first mapping lazily creates a shared_tag_mapping service.
 */
export function useShareMappings() {
    const {data: services} = useTaggingServices()
    const mutations = useTaggingMutations()

    const mappingServices = useMemo(
        () =>
            (services ?? []).filter(
                (s): s is SharedTagMappingServiceDetail => s.service_type === 'shared_tag_mapping',
            ),
        [services],
    )

    const forShare = (incomingShareId: string): ShareMapping[] =>
        mappingServices.flatMap((svc) =>
            svc.mappings
                .filter((m) => m.incoming_share_id === incomingShareId)
                .map((m) => ({serviceId: svc.id, ruleId: m.id, assign_tag: m.assign_tag, is_broken: m.is_broken})),
        )

    const addMapping = async (incomingShareId: string, assignTag: string) => {
        // Enforce a single mapping per share: drop any existing ones first.
        for (const existing of forShare(incomingShareId)) {
            await mutations.deleteMapping.mutateAsync({serviceId: existing.serviceId, ruleId: existing.ruleId})
        }
        let serviceId = mappingServices[0]?.id
        if (!serviceId) {
            const created = await mutations.create.mutateAsync({service_type: 'shared_tag_mapping'})
            serviceId = created.id
        }
        await mutations.addMapping.mutateAsync({serviceId, incoming_share_id: incomingShareId, assign_tag: assignTag})
    }

    const removeMapping = (serviceId: string, ruleId: string) => mutations.deleteMapping.mutateAsync({serviceId, ruleId})

    const isBusy =
        mutations.create.isPending || mutations.addMapping.isPending || mutations.deleteMapping.isPending

    return {mappingServices, forShare, addMapping, removeMapping, isBusy}
}
