import {Network, Ticket, UserCircle} from 'lucide-react'
import {useNavigate} from 'react-router-dom'
import {toast} from 'sonner'
import {InvitesManager} from '@/components/admin/InvitesManager'
import {Button} from '@/components/ui/button'
import {useAdminInviteRevoke, useAllAdminInvites} from '@/hooks/useAdmin'
import {useInviteMutations, useRegistrationInfo} from '@/hooks/useInvites'
import {useAuthStore} from '@/stores/auth'
import {useQueryClient} from '@tanstack/react-query'
import {queryKeys} from '@/lib/constants'
import {apiErrorMessage} from '@/api/client'

/**
 * Admin invites (feature 24) — every local invite, **grouped by the user who created it**, with revoke.
 * The admin can also mint their own. In **resolver mode** invites live on the resolver, not this
 * backend, so the local list would be empty/misleading — we show a pointer to the fleet dashboard and
 * the user's own profile instead (feature 25).
 */
export function InvitesTab() {
    const isResolver = useAuthStore((s) => s.isResolver)
    if (isResolver) return <ResolverModeNotice/>
    return <LocalInvites/>
}

function ResolverModeNotice() {
    const navigate = useNavigate()
    const isAdmin = useAuthStore((s) => s.user?.is_admin ?? false)
    return (
        <div className="mx-auto max-w-2xl">
            <div className="flex items-start gap-3 rounded-md border border-border bg-muted/40 px-4 py-3.5 text-sm">
                <Ticket className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"/>
                <div className="space-y-3">
                    <p>
                        This instance is fronted by a <span className="font-medium">resolver</span>, so invites are
                        managed fleet-wide on the resolver — not per-backend here.
                    </p>
                    <div className="flex flex-wrap gap-2">
                        {isAdmin && (
                            <Button variant="outline" size="sm" className="h-8 gap-1.5"
                                    onClick={() => navigate('/admin/resolver')}>
                                <Network className="h-3.5 w-3.5"/>
                                Fleet dashboard
                            </Button>
                        )}
                        <Button variant="outline" size="sm" className="h-8 gap-1.5"
                                onClick={() => navigate('/settings')}>
                            <UserCircle className="h-3.5 w-3.5"/>
                            Your invites (Profile)
                        </Button>
                    </div>
                </div>
            </div>
        </div>
    )
}

function LocalInvites() {
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
