import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {
    cancelJob,
    createAdminUser,
    deleteAdminUser,
    forceReconcileShare,
    getConsistencyCheck,
    getErroredShares,
    getInstanceHealth,
    getInstanceStats,
    getStaleJobs,
    getUserShares,
    getUserStats,
    getUserStorageAudit,
    listAdminJobs,
    listAdminUsers,
    listFederationInstances,
    type ListJobsParams,
    resetJob,
    updateAdminUser,
    wakeUserPipeline,
} from '@/api/admin'
import {queryKeys} from '@/lib/constants'

export function useInstanceHealth() {
    return useQuery({
        queryKey: queryKeys.adminInstanceHealth(),
        queryFn: getInstanceHealth,
        refetchInterval: 30_000,
    })
}

export function useInstanceStats() {
    return useQuery({
        queryKey: queryKeys.adminStats(),
        queryFn: getInstanceStats,
        refetchInterval: 60_000,
    })
}

export function useConsistencyCheck() {
    return useQuery({
        queryKey: queryKeys.adminConsistency(),
        queryFn: getConsistencyCheck,
        refetchInterval: 60_000,
    })
}

export function useAdminUsers() {
    return useQuery({
        queryKey: queryKeys.adminUsers(),
        queryFn: listAdminUsers,
    })
}

export function useUserStats(id: string | null) {
    return useQuery({
        queryKey: queryKeys.adminUserStats(id ?? ''),
        queryFn: () => getUserStats(id!),
        enabled: !!id,
    })
}

export function useUserShares(id: string | null) {
    return useQuery({
        queryKey: queryKeys.adminUserShares(id ?? ''),
        queryFn: () => getUserShares(id!),
        enabled: !!id,
    })
}

export function useUserStorageAudit(id: string | null) {
    return useQuery({
        queryKey: queryKeys.adminUserStorageAudit(id ?? ''),
        queryFn: () => getUserStorageAudit(id!),
        enabled: !!id,
        staleTime: 60_000,
    })
}

export function useAdminJobs(params: ListJobsParams = {}) {
    return useQuery({
        queryKey: queryKeys.adminJobs(params),
        queryFn: () => listAdminJobs(params),
        refetchInterval: 15_000,
    })
}

export function useStaleJobs() {
    return useQuery({
        queryKey: queryKeys.adminStaleJobs(),
        queryFn: getStaleJobs,
        refetchInterval: 15_000,
    })
}

export function useErroredShares() {
    return useQuery({
        queryKey: queryKeys.adminErroredShares(),
        queryFn: getErroredShares,
        refetchInterval: 30_000,
    })
}

export function useFederationInstances() {
    return useQuery({
        queryKey: queryKeys.adminFederationInstances(),
        queryFn: listFederationInstances,
    })
}

export function useAdminUserMutations() {
    const queryClient = useQueryClient()

    const invalidateUsers = () => queryClient.invalidateQueries({queryKey: ['admin', 'users']})

    const create = useMutation({
        mutationFn: createAdminUser,
        onSuccess: invalidateUsers,
    })

    const update = useMutation({
        mutationFn: ({id, body}: {
            id: string
            body: { display_name?: string; is_admin?: boolean; storage_quota_bytes?: number | null }
        }) => updateAdminUser(id, body),
        onSuccess: invalidateUsers,
    })

    const remove = useMutation({
        mutationFn: deleteAdminUser,
        onSuccess: invalidateUsers,
    })

    const wake = useMutation({
        mutationFn: wakeUserPipeline,
    })

    return {create, update, remove, wake}
}

export function useAdminJobMutations() {
    const queryClient = useQueryClient()

    const invalidateJobs = () => queryClient.invalidateQueries({queryKey: ['admin', 'jobs']})

    const reset = useMutation({
        mutationFn: resetJob,
        onSuccess: invalidateJobs,
    })

    const cancel = useMutation({
        mutationFn: cancelJob,
        onSuccess: invalidateJobs,
    })

    return {reset, cancel}
}

export function useAdminShareMutations() {
    const queryClient = useQueryClient()

    const invalidateShares = () => queryClient.invalidateQueries({queryKey: ['admin', 'shares']})

    const forceReconcile = useMutation({
        mutationFn: forceReconcileShare,
        onSuccess: invalidateShares,
    })

    return {forceReconcile}
}
