import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {
    addMapping,
    addRule,
    addSegment,
    createService,
    deleteMapping,
    deleteRule,
    deleteSegment,
    deleteService,
    getService,
    listServices,
    reorderRules,
    reorderServices,
    updateRule,
    updateService,
} from '@/api/tagging'
import {queryKeys} from '@/lib/constants'
import type {RulePredicate} from '@/lib/types'

export function useTaggingServices() {
    return useQuery({queryKey: queryKeys.taggingServices(), queryFn: listServices})
}

export function useTaggingService(id: string | null) {
    return useQuery({
        queryKey: queryKeys.taggingService(id ?? ''),
        enabled: !!id,
        queryFn: () => getService(id!),
    })
}

/**
 * Mutations for the tagging pipeline. Every change can affect assigned tags
 * (re-evaluated asynchronously by the backend pipeline), so we invalidate the
 * tagging, tags and pictures caches.
 */
export function useTaggingMutations() {
    const qc = useQueryClient()
    const invalidate = () => {
        void qc.invalidateQueries({queryKey: ['tagging']})
        void qc.invalidateQueries({queryKey: ['tags']})
        void qc.invalidateQueries({queryKey: ['pictures']})
    }

    return {
        create: useMutation({mutationFn: createService, onSuccess: invalidate}),
        update: useMutation({
            mutationFn: (vars: {
                id: string
                body: { name?: string; enabled?: boolean; requires?: string[]; excludes?: string[] }
            }) => updateService(vars.id, vars.body),
            onSuccess: invalidate,
        }),
        remove: useMutation({
            mutationFn: (vars: { id: string; promoteTags: boolean }) => deleteService(vars.id, vars.promoteTags),
            onSuccess: invalidate,
        }),
        reorder: useMutation({mutationFn: reorderServices, onSuccess: invalidate}),
        addMapping: useMutation({
            mutationFn: (vars: { serviceId: string; incoming_share_id: string; assign_tag: string }) =>
                addMapping(vars.serviceId, {incoming_share_id: vars.incoming_share_id, assign_tag: vars.assign_tag}),
            onSuccess: invalidate,
        }),
        deleteMapping: useMutation({
            mutationFn: (vars: { serviceId: string; ruleId: string }) => deleteMapping(vars.serviceId, vars.ruleId),
            onSuccess: invalidate,
        }),
        addRule: useMutation({
            mutationFn: (vars: { serviceId: string; predicate: RulePredicate; assign_tag: string }) =>
                addRule(vars.serviceId, {predicate: vars.predicate, assign_tag: vars.assign_tag}),
            onSuccess: invalidate,
        }),
        editRule: useMutation({
            mutationFn: (vars: { serviceId: string; ruleId: string; predicate: RulePredicate; assign_tag: string }) =>
                updateRule(vars.serviceId, vars.ruleId, {predicate: vars.predicate, assign_tag: vars.assign_tag}),
            onSuccess: invalidate,
        }),
        reorderRules: useMutation({
            mutationFn: (vars: { serviceId: string; orderedIds: string[] }) =>
                reorderRules(vars.serviceId, vars.orderedIds),
            onSuccess: invalidate,
        }),
        deleteRule: useMutation({
            mutationFn: (vars: { serviceId: string; ruleId: string }) => deleteRule(vars.serviceId, vars.ruleId),
            onSuccess: invalidate,
        }),
        addSegment: useMutation({
            mutationFn: (vars: {
                serviceId: string
                name: string
                date_start: string
                date_end: string
                assign_tag: string
                parent_segment_id?: string
            }) =>
                addSegment(vars.serviceId, {
                    name: vars.name,
                    date_start: vars.date_start,
                    date_end: vars.date_end,
                    assign_tag: vars.assign_tag,
                    parent_segment_id: vars.parent_segment_id,
                }),
            onSuccess: invalidate,
        }),
        deleteSegment: useMutation({
            mutationFn: (vars: { serviceId: string; segmentId: string }) => deleteSegment(vars.serviceId, vars.segmentId),
            onSuccess: invalidate,
        }),
    }
}
