import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {
    getBackends,
    getConfigMatrix,
    getNextBackend,
    getOverview,
    getRoutines,
    getSettings,
    listInvites,
    mintInvite,
    patchConfigMatrix,
    patchSetting,
    resetSetting,
    revokeInvite,
    setCapacity,
    triggerRoutine,
} from '@/api/resolverAdmin'
import {queryKeys} from '@/lib/constants'
import {useResolverAuthStore} from '@/stores/resolverAuth'

/** Whether the operator has a live resolver session (drives the dashboard vs. login gate). */
export function useResolverSession() {
    return useResolverAuthStore((s) => s.sessionToken)
}

export function useResolverOverview(opts: { refetchInterval?: number | false } = {}) {
    return useQuery({
        queryKey: queryKeys.resolverAdmin.overview(),
        queryFn: getOverview,
        refetchInterval: opts.refetchInterval ?? false,
    })
}

export function useResolverBackends(opts: { refetchInterval?: number | false } = {}) {
    return useQuery({
        queryKey: queryKeys.resolverAdmin.backends(),
        queryFn: getBackends,
        refetchInterval: opts.refetchInterval ?? false,
    })
}

export function useNextBackend(opts: { refetchInterval?: number | false } = {}) {
    return useQuery({
        queryKey: [...queryKeys.resolverAdmin.backends(), 'next'],
        queryFn: getNextBackend,
        refetchInterval: opts.refetchInterval ?? false,
    })
}

export function useResolverSettings() {
    return useQuery({
        queryKey: queryKeys.resolverAdmin.settings(),
        queryFn: getSettings,
    })
}

export function useResolverSettingMutations() {
    const qc = useQueryClient()
    const invalidate = () => qc.invalidateQueries({queryKey: queryKeys.resolverAdmin.settings()})
    const patch = useMutation({
        mutationFn: ({key, value}: { key: string; value: unknown }) => patchSetting(key, value),
        onSuccess: (data) => qc.setQueryData(queryKeys.resolverAdmin.settings(), data),
    })
    const reset = useMutation({
        mutationFn: (key: string) => resetSetting(key),
        onSuccess: (data) => qc.setQueryData(queryKeys.resolverAdmin.settings(), data),
    })
    return {patch, reset, invalidate}
}

export function useResolverInvites() {
    return useQuery({
        queryKey: queryKeys.resolverAdmin.invites(),
        queryFn: listInvites,
    })
}

export function useResolverInviteMutations() {
    const qc = useQueryClient()
    const invalidate = () => qc.invalidateQueries({queryKey: queryKeys.resolverAdmin.invites()})
    const mint = useMutation({mutationFn: mintInvite, onSuccess: invalidate})
    const revoke = useMutation({mutationFn: revokeInvite, onSuccess: invalidate})
    return {mint, revoke}
}

export function useResolverCapacityMutation() {
    const qc = useQueryClient()
    return useMutation({
        mutationFn: ({backDomain, body}: {
            backDomain: string
            body: { accepting_registrations: boolean; max_users: number | null }
        }) => setCapacity(backDomain, body),
        onSuccess: () => {
            void qc.invalidateQueries({queryKey: queryKeys.resolverAdmin.backends()})
            void qc.invalidateQueries({queryKey: queryKeys.resolverAdmin.overview()})
        },
    })
}

export function useResolverRoutines(opts: { refetchInterval?: number | false } = {}) {
    return useQuery({
        queryKey: queryKeys.resolverAdmin.routines(),
        queryFn: getRoutines,
        refetchInterval: opts.refetchInterval ?? false,
    })
}

export function useTriggerResolverRoutine() {
    const qc = useQueryClient()
    return useMutation({
        mutationFn: triggerRoutine,
        onSuccess: () => setTimeout(() => qc.invalidateQueries({queryKey: queryKeys.resolverAdmin.routines()}), 1000),
    })
}

export function useConfigMatrix() {
    return useQuery({
        queryKey: queryKeys.resolverAdmin.configMatrix(),
        queryFn: getConfigMatrix,
    })
}

export function useConfigMatrixPatch() {
    const qc = useQueryClient()
    return useMutation({
        mutationFn: patchConfigMatrix,
        onSuccess: () => qc.invalidateQueries({queryKey: queryKeys.resolverAdmin.configMatrix()}),
    })
}
