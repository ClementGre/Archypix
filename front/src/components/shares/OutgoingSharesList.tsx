import {useMemo} from 'react'
import {Ban, Loader2, Send} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useOutgoingShares, useShareMutations} from '@/hooks/useShares'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import type {ShareResponse} from '@/lib/types'
import {ShareStatusBadge} from './ShareStatusBadge'
import {CreateShareDialog} from './CreateShareDialog'

const REVOCABLE = new Set(['pending', 'pending_first_announcement', 'active', 'errored'])

export function OutgoingSharesList() {
    const {data: shares, isPending, isError, error} = useOutgoingShares()
    const {revoke} = useShareMutations()
    const {update} = useGalleryParams()

    // Group shares by tag so one tag shared to many recipients is shown together.
    const groups = useMemo(() => {
        const map = new Map<string, ShareResponse[]>()
        for (const s of shares ?? []) {
            const list = map.get(s.tag_path) ?? []
            list.push(s)
            map.set(s.tag_path, list)
        }
        return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))
    }, [shares])

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
    if (!groups.length) {
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
                {groups.map(([tag, recipients]) => (
                    <div key={tag} className="rounded-md border border-border p-2.5">
                        <button
                            onClick={() => update({tag})}
                            className="flex w-full items-center justify-between gap-2 text-left"
                            title="Filter to this tag"
                        >
                            <span className="truncate text-sm font-medium hover:text-primary">{TagPath.toDisplay(tag)}</span>
                            <span className="shrink-0 text-xs text-muted-foreground">{recipients.length}</span>
                        </button>

                        <div className="mt-2 space-y-1.5">
                            {recipients.map((share) => (
                                <div key={share.id} className="flex items-center justify-between gap-2">
                  <span className="min-w-0 truncate text-xs text-muted-foreground">
                    → @{share.recipient_username}:{share.recipient_instance}
                  </span>
                                    <div className="flex shrink-0 items-center gap-1.5">
                                        <ShareStatusBadge status={share.status}/>
                                        {REVOCABLE.has(share.status) && (
                                            <ConfirmDialog
                                                title="Revoke this share?"
                                                description={`Stop sharing with @${share.recipient_username}:${share.recipient_instance}. Their access and the shared pictures are removed immediately.`}
                                                confirmLabel="Revoke"
                                                destructive
                                                onConfirm={() =>
                                                    revoke.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})
                                                }
                                                trigger={
                                                    <Button
                                                        size="icon"
                                                        variant="ghost"
                                                        className="h-6 w-6 text-muted-foreground hover:text-destructive"
                                                        title="Revoke"
                                                        disabled={revoke.isPending}
                                                    >
                                                        <Ban className="h-3.5 w-3.5"/>
                                                    </Button>
                                                }
                                            />
                                        )}
                                    </div>
                                </div>
                            ))}
                        </div>
                    </div>
                ))}
            </div>
        </>
    )
}
