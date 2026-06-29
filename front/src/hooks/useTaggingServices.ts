import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {createService, deleteService, getService, listServices, reorderServices, replaceConfig, updateService,} from '@/api/tagging'
import {queryKeys} from '@/lib/constants'
import {invalidatePicturesAndTags} from '@/lib/invalidation'
import type {ServiceConfig} from '@/lib/types'

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
 * tagging, tags and pictures caches (now and again after the settle window, §9).
 */
export function useTaggingMutations() {
    const qc = useQueryClient()
    const invalidate = () => {
        void qc.invalidateQueries({queryKey: ['tagging']})
        invalidatePicturesAndTags(qc)
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
        replaceConfig: useMutation({
            mutationFn: (vars: { id: string; config: ServiceConfig }) => replaceConfig(vars.id, vars.config),
            onSuccess: invalidate,
        }),
        remove: useMutation({
            mutationFn: (vars: { id: string; promoteTags: boolean }) => deleteService(vars.id, vars.promoteTags),
            onSuccess: invalidate,
        }),
        reorder: useMutation({mutationFn: reorderServices, onSuccess: invalidate}),
    }
}
