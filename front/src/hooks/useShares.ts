import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {acceptIncomingShare, listIncomingShares, listOutgoingShares, rejectIncomingShare, revokeOutgoingShare,} from '@/api/shares'
import {queryKeys} from '@/lib/constants'

export function useIncomingShares() {
    return useQuery({
        queryKey: queryKeys.incomingShares(),
        queryFn: listIncomingShares,
    })
}

export function useOutgoingShares() {
    return useQuery({
        queryKey: queryKeys.outgoingShares(),
        queryFn: listOutgoingShares,
    })
}

export function useShareMutations() {
    const queryClient = useQueryClient()

    const invalidateShares = () => {
        void queryClient.invalidateQueries({queryKey: ['shares']})
    }

    const accept = useMutation({
        mutationFn: acceptIncomingShare,
        onSuccess: invalidateShares,
    })

    const reject = useMutation({
        mutationFn: rejectIncomingShare,
        onSuccess: invalidateShares,
    })

    const revoke = useMutation({
        mutationFn: revokeOutgoingShare,
        onSuccess: invalidateShares,
    })

    return {accept, reject, revoke}
}
