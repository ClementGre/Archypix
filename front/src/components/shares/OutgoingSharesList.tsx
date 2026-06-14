import {Ban, Loader2, Send} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {useOutgoingShares, useShareMutations} from '@/hooks/useShares'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {ShareStatusBadge} from './ShareStatusBadge'
import {CreateShareDialog} from './CreateShareDialog'

const REVOCABLE = new Set(['pending', 'pending_first_announcement', 'active', 'errored'])

export function OutgoingSharesList() {
    const {data: shares, isPending, isError, error} = useOutgoingShares()
    const {revoke} = useShareMutations()
    const {update} = useGalleryParams()

    const header = (
        <div className="flex items-center justify-between px-2 pt-2">
            <span className="text-xs font-medium text-muted-foreground">Outgoing</span>
            <CreateShareDialog/>
        </div>
    )

    if (isPending) {
        return (
            <>
                {header}
                <div className="flex items-center justify-center py-6 text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin"/>
                </div>
            </>
        )
    }
    if (isError) {
        return (
            <>
                {header}
                <p className="px-3 py-4 text-xs text-muted-foreground">{apiErrorMessage(error)}</p>
            </>
        )
    }
    if (!shares.length) {
        return (
            <>
                {header}
                <div className="flex flex-col items-center gap-2 px-3 py-8 text-center text-xs text-muted-foreground">
                    <Send className="h-6 w-6"/>
                    You haven&apos;t shared anything yet.
                </div>
            </>
        )
    }

    return (
        <>
            {header}
            <div className="space-y-2 p-2">
                {shares.map((share) => (
                    <div key={share.id} className="rounded-md border border-border p-2.5">
                        <div className="flex items-center justify-between gap-2">
                            <button
                                onClick={() => update({tag: share.tag_path})}
                                className="truncate text-left text-sm font-medium hover:text-primary"
                                title="Filter to this tag"
                            >
                                {TagPath.toDisplay(share.tag_path)}
                            </button>
                            <ShareStatusBadge status={share.status}/>
                        </div>
                        <p className="mt-1 truncate text-xs text-muted-foreground">
                            → @{share.recipient_username}:{share.recipient_instance}
                        </p>
                        {REVOCABLE.has(share.status) && (
                            <div className="mt-2">
                                <Button
                                    size="sm"
                                    variant="outline"
                                    className="h-7 gap-1 px-2"
                                    disabled={revoke.isPending}
                                    onClick={() => revoke.mutate(share.id, {onError: (e) => alert(apiErrorMessage(e))})}
                                >
                                    <Ban className="h-3.5 w-3.5"/> Revoke
                                </Button>
                            </div>
                        )}
                    </div>
                ))}
            </div>
        </>
    )
}
