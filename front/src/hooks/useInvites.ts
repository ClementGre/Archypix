import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {getInvitations, getRegistrationInfo, listInvites, mintInvite, revokeInvite} from '@/api/invites'
import {queryKeys} from '@/lib/constants'
import {useAuthStore} from '@/stores/auth'

/** Effective registration mode where this user's signups land (their instance / the resolver). */
export function useRegistrationInfo() {
    const instance = useAuthStore((s) => s.instance)
    return useQuery({
        queryKey: ['registration-info', instance],
        queryFn: () => getRegistrationInfo(instance ?? undefined),
        staleTime: 5 * 60_000,
    })
}

export function useInvites() {
    return useQuery({
        queryKey: queryKeys.invites(),
        queryFn: listInvites,
    })
}

export function useInvitations() {
    return useQuery({
        queryKey: queryKeys.invitations(),
        queryFn: getInvitations,
    })
}

export function useInviteMutations() {
    const qc = useQueryClient()
    const invalidate = () => qc.invalidateQueries({queryKey: queryKeys.invites()})
    const mint = useMutation({mutationFn: mintInvite, onSuccess: invalidate})
    const revoke = useMutation({mutationFn: revokeInvite, onSuccess: invalidate})
    return {mint, revoke}
}
