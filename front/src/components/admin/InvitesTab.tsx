import {toast} from 'sonner'
import {InvitesManager} from '@/components/admin/InvitesManager'
import {useAllAdminInvites, useAdminInviteRevoke} from '@/hooks/useAdmin'
import {useInviteMutations, useRegistrationInfo} from '@/hooks/useInvites'
import {useAuthStore} from '@/stores/auth'
import {useQueryClient} from '@tanstack/react-query'
import {queryKeys} from '@/lib/constants'
import {apiErrorMessage} from '@/api/client'

/**
 * Admin invites (feature 24) — every local invite, **grouped by the user who created it**, with revoke.
 * The admin can also mint their own. (In resolver mode invites live on the resolver, so this list is
 * empty and invites are managed from the fleet dashboard.)
 */
export function InvitesTab() {
    const {data: invites, isLoading} = useAllAdminInvites()
    const {data: info} = useRegistrationInfo()
    const {mint} = useInviteMutations()
    const revoke = useAdminInviteRevoke()
    const isAdmin = useAuthStore((s) => s.user?.is_admin ?? false)
    const queryClient = useQueryClient()
    const canMint = info ? info.mode !== 'admin_invite' || isAdmin : isAdmin

    return (
        <div className="mx-auto max-w-3xl">
            <InvitesManager
                invites={invites}
                isLoading={isLoading}
                mode={info?.mode}
                canMint={canMint}
                groupByCreator
                onMint={async (body) => {
                    const inv = await mint.mutateAsync({max_uses: body.max_uses, expires_in_days: body.expires_in_days})
                    void queryClient.invalidateQueries({queryKey: queryKeys.adminInvites()})
                    return inv
                }}
                onRevoke={async (code) => {
                    try {
                        await revoke.mutateAsync(code)
                        toast.success('Invite revoked')
                    } catch (e) {
                        toast.error('Could not revoke invite', {description: apiErrorMessage(e)})
                    }
                }}
            />
        </div>
    )
}
