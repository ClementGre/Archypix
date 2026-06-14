import {Check, Images, Inbox, Loader2, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {useIncomingShares, useShareMutations} from '@/hooks/useShares'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import type {IncomingShareResponse} from '@/lib/types'
import {ShareStatusBadge} from './ShareStatusBadge'

function sharedToMeTag(share: IncomingShareResponse): string {
    const label = `${share.sender_username}@${share.sender_instance}`.replace(/@/g, '_AT_').replace(/\./g, '_DOT_')
    return `SharedToMe.${label}`
}

export function IncomingSharesList() {
    const {data: shares, isPending, isError, error} = useIncomingShares()
    const {accept, reject} = useShareMutations()
    const {update} = useGalleryParams()

    if (isPending) {
        return (
            <div className="flex items-center justify-center py-6 text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin"/>
            </div>
        )
    }
    if (isError) return <p className="px-3 py-4 text-xs text-muted-foreground">{apiErrorMessage(error)}</p>
    if (!shares.length) {
        return (
            <div className="flex flex-col items-center gap-2 px-3 py-8 text-center text-xs text-muted-foreground">
                <Inbox className="h-6 w-6"/>
                No incoming shares.
            </div>
        )
    }

    return (
        <div className="space-y-2 p-2">
            {shares.map((share) => (
                <div key={share.id} className="rounded-md border border-border p-2.5">
                    <div className="flex items-center justify-between gap-2">
            <span className="truncate text-sm font-medium">
              @{share.sender_username}
                <span className="text-muted-foreground">:{share.sender_instance}</span>
            </span>
                        <ShareStatusBadge status={share.status}/>
                    </div>
                    <div className="mt-2 flex flex-wrap gap-1.5">
                        {share.status === 'pending' && (
                            <>
                                <Button
                                    size="sm"
                                    className="h-7 gap-1 px-2"
                                    disabled={accept.isPending}
                                    onClick={() => accept.mutate(share.id, {onError: (e) => alert(apiErrorMessage(e))})}
                                >
                                    <Check className="h-3.5 w-3.5"/> Accept
                                </Button>
                                <Button
                                    size="sm"
                                    variant="outline"
                                    className="h-7 gap-1 px-2"
                                    disabled={reject.isPending}
                                    onClick={() => reject.mutate(share.id, {onError: (e) => alert(apiErrorMessage(e))})}
                                >
                                    <X className="h-3.5 w-3.5"/> Reject
                                </Button>
                            </>
                        )}
                        {share.status === 'active' && (
                            <Button
                                size="sm"
                                variant="ghost"
                                className="h-7 gap-1 px-2 text-muted-foreground"
                                onClick={() => update({tag: sharedToMeTag(share), scope: 'all'})}
                            >
                                <Images className="h-3.5 w-3.5"/> View photos
                            </Button>
                        )}
                    </div>
                </div>
            ))}
        </div>
    )
}
