import {toast} from 'sonner'
import {InvitesManager} from '@/components/admin/InvitesManager'
import {useResolverBackends, useResolverInvites, useResolverInviteMutations, useResolverSettings} from '@/hooks/useResolverAdmin'
import {apiErrorMessage} from '@/api/client'
import type {RegistrationMode} from '@/lib/types'

/** Fleet-wide invite management with instance-pinning (feature 24 §5). */
export function ResolverInvitesTab() {
    const {data: invites, isLoading} = useResolverInvites()
    const {data: backends} = useResolverBackends()
    const {data: settings} = useResolverSettings()
    const {mint, revoke} = useResolverInviteMutations()

    // The resolver's registration_mode field drives the mint form (tracking vs. gated).
    const modeField = settings?.find((f) => f.key === 'registration_mode')
    const mode = (typeof modeField?.value === 'string' ? modeField.value : 'open') as RegistrationMode

    return (
        <div className="mx-auto max-w-3xl">
            <InvitesManager
                invites={invites}
                isLoading={isLoading}
                mode={mode}
                canMint
                groupByCreator
                pinOptions={(backends ?? []).map((b) => b.back_domain)}
                showPin
                onMint={(body) =>
                    mint.mutateAsync({
                        max_uses: body.max_uses,
                        expires_in_days: body.expires_in_days,
                        instance_pin: body.instance_pin ?? null,
                    })
                }
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
