import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {
    cancelJob,
    createAdminUser,
    deleteAdminUser,
    forceReconcileShare,
    getAdminRoutines,
    getAdminSettings,
    getConsistencyCheck,
    listAllAdminInvites,
    revokeAnyInvite,
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
    patchAdminSetting,
    resetAdminSetting,
    resetJob,
    triggerAdminRoutine,
    updateAdminUser,
    wakeUserPipeline,
} from '@/api/admin'
import {queryKeys} from '@/lib/constants'
import {useAdminClient} from '@/api/adminClient'

// Every hook resolves its transport (direct `apiClient` or a resolver-proxy client) + a cache `scope`
// from `useAdminClient()`, so the same admin components render for `/admin` and a fleet drill-down
// without collisions. `scope` is appended to each query key (prefix-invalidation still works).

export function useInstanceHealth() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminInstanceHealth(), scope],
        queryFn: () => getInstanceHealth(client),
        refetchInterval: 30_000,
    })
}

export function useInstanceStats() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminStats(), scope],
        queryFn: () => getInstanceStats(client),
        refetchInterval: 60_000,
    })
}

export function useConsistencyCheck() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminConsistency(), scope],
        queryFn: () => getConsistencyCheck(client),
        refetchInterval: 60_000,
    })
}

export function useAdminUsers() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminUsers(), scope],
        queryFn: () => listAdminUsers(client),
    })
}

export function useUserStats(id: string | null) {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminUserStats(id ?? ''), scope],
        queryFn: () => getUserStats(client, id!),
        enabled: !!id,
    })
}

export function useUserShares(id: string | null) {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminUserShares(id ?? ''), scope],
        queryFn: () => getUserShares(client, id!),
        enabled: !!id,
    })
}

export function useUserStorageAudit(id: string | null) {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminUserStorageAudit(id ?? ''), scope],
        queryFn: () => getUserStorageAudit(client, id!),
        enabled: !!id,
        staleTime: 60_000,
    })
}

export function useAdminJobs(params: ListJobsParams = {}) {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminJobs(params), scope],
        queryFn: () => listAdminJobs(client, params),
        refetchInterval: 15_000,
    })
}

export function useStaleJobs() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminStaleJobs(), scope],
        queryFn: () => getStaleJobs(client),
        refetchInterval: 15_000,
    })
}

export function useErroredShares() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminErroredShares(), scope],
        queryFn: () => getErroredShares(client),
        refetchInterval: 30_000,
    })
}

export function useFederationInstances() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminFederationInstances(), scope],
        queryFn: () => listFederationInstances(client),
    })
}

export function useAdminUserMutations() {
    const {client} = useAdminClient()
    const queryClient = useQueryClient()
    const invalidateUsers = () => queryClient.invalidateQueries({queryKey: ['admin', 'users']})

    const create = useMutation({
        mutationFn: (body: Parameters<typeof createAdminUser>[1]) => createAdminUser(client, body),
        onSuccess: invalidateUsers,
    })
    const update = useMutation({
        mutationFn: ({id, body}: {
            id: string
            body: { display_name?: string; is_admin?: boolean; storage_quota_bytes?: number | null }
        }) => updateAdminUser(client, id, body),
        onSuccess: invalidateUsers,
    })
    const remove = useMutation({
        mutationFn: (id: string) => deleteAdminUser(client, id),
        onSuccess: invalidateUsers,
    })
    const wake = useMutation({mutationFn: (id: string) => wakeUserPipeline(client, id)})

    return {create, update, remove, wake}
}

export function useAdminJobMutations() {
    const {client} = useAdminClient()
    const queryClient = useQueryClient()
    const invalidateJobs = () => queryClient.invalidateQueries({queryKey: ['admin', 'jobs']})

    const reset = useMutation({mutationFn: (id: string) => resetJob(client, id), onSuccess: invalidateJobs})
    const cancel = useMutation({mutationFn: (id: string) => cancelJob(client, id), onSuccess: invalidateJobs})
    return {reset, cancel}
}

export function useAdminShareMutations() {
    const {client} = useAdminClient()
    const queryClient = useQueryClient()
    const invalidateShares = () => queryClient.invalidateQueries({queryKey: ['admin', 'shares']})
    const forceReconcile = useMutation({mutationFn: (id: string) => forceReconcileShare(client, id), onSuccess: invalidateShares})
    return {forceReconcile}
}

// ── Runtime settings + routines (feature 23/24) ──────────────────────────────────

export function useAdminSettings() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminSettings(), scope],
        queryFn: () => getAdminSettings(client),
    })
}

export function useAdminSettingMutations() {
    const {client, scope} = useAdminClient()
    const queryClient = useQueryClient()
    const patch = useMutation({
        mutationFn: ({key, value}: { key: string; value: unknown }) => patchAdminSetting(client, key, value),
        onSuccess: (data) => {
            queryClient.setQueryData([...queryKeys.adminSettings(), scope], data)
            void queryClient.invalidateQueries({queryKey: queryKeys.adminRoutines()})
        },
    })
    const reset = useMutation({
        mutationFn: (key: string) => resetAdminSetting(client, key),
        onSuccess: (data) => {
            queryClient.setQueryData([...queryKeys.adminSettings(), scope], data)
            void queryClient.invalidateQueries({queryKey: queryKeys.adminRoutines()})
        },
    })
    return {patch, reset}
}

export function useAdminRoutines(opts: { refetchInterval?: number | false } = {}) {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminRoutines(), scope],
        queryFn: () => getAdminRoutines(client),
        refetchInterval: opts.refetchInterval ?? false,
    })
}

export function useTriggerRoutine() {
    const {client} = useAdminClient()
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: (name: string) => triggerAdminRoutine(client, name),
        onSuccess: () => setTimeout(() => queryClient.invalidateQueries({queryKey: queryKeys.adminRoutines()}), 1000),
    })
}

// ── Invites (all local, grouped by creator) ──────────────────────────────────────

export function useAllAdminInvites() {
    const {client, scope} = useAdminClient()
    return useQuery({
        queryKey: [...queryKeys.adminInvites(), scope],
        queryFn: () => listAllAdminInvites(client),
    })
}

export function useAdminInviteRevoke() {
    const {client} = useAdminClient()
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: (code: string) => revokeAnyInvite(client, code),
        onSuccess: () => queryClient.invalidateQueries({queryKey: queryKeys.adminInvites()}),
    })
}
