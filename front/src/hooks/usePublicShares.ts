import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {queryKeys} from '@/lib/constants'
import type {PublicShareBody} from '@/api/publicShares'
import {createPublicShare, listPublicShares, revokePublicShare, trashContributions, updatePublicShare,} from '@/api/publicShares'

/** The owner's public links (feature 27 management surface). */
export function usePublicShares() {
    return useQuery({
        queryKey: queryKeys.publicShares(),
        queryFn: listPublicShares,
    })
}

export function usePublicShareMutations() {
    const qc = useQueryClient()
    const invalidate = () => void qc.invalidateQueries({queryKey: queryKeys.publicShares()})

    const create = useMutation({
        mutationFn: (body: PublicShareBody) => createPublicShare(body),
        onSuccess: invalidate,
    })
    const update = useMutation({
        mutationFn: ({id, body}: { id: string; body: PublicShareBody }) => updatePublicShare(id, body),
        onSuccess: invalidate,
    })
    const revoke = useMutation({
        mutationFn: ({id, cascade, trash}: { id: string; cascade: boolean; trash: boolean }) =>
            revokePublicShare(id, {cascade_derived: cascade, trash_contributions: trash}),
        onSuccess: invalidate,
    })
    const trashContribs = useMutation({
        mutationFn: ({id, contributor}: { id: string; contributor?: string | null }) =>
            trashContributions(id, contributor),
        onSuccess: invalidate,
    })

    return {create, update, revoke, trashContribs}
}
