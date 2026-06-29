import {toast} from 'sonner'
import {Dialog, DialogContent, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {useIncomingShares} from '@/hooks/useShares'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {SharedTagMappingConfig} from '@/lib/types'

interface NewMappingDialogProps {
    open: boolean
    onOpenChange: (open: boolean) => void
    /** Incoming-share ids that already have a mapping service (one per share). */
    mappedShareIds: string[]
    onCreated: (serviceId: string) => void
}

/** Pick an active incoming share to create a one-per-share shared-tag-mapping service. */
export function NewMappingDialog({open, onOpenChange, mappedShareIds, onCreated}: NewMappingDialogProps) {
    const {data: shares} = useIncomingShares()
    const {create} = useTaggingMutations()

    const candidates = (shares ?? []).filter((s) => s.status === 'active' && !mappedShareIds.includes(s.id))

    const pick = (incomingShareId: string) => {
        const config: SharedTagMappingConfig = {incoming_share_id: incomingShareId, assign_tags: []}
        create.mutate(
            {service_type: 'shared_tag_mapping', config},
            {
                onSuccess: (svc) => {
                    onOpenChange(false)
                    onCreated(svc.id)
                },
                onError: (err) => toast.error(apiErrorMessage(err)),
            },
        )
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent>
                <DialogHeader>
                    <DialogTitle>Map an incoming share</DialogTitle>
                </DialogHeader>
                {candidates.length === 0 ? (
                    <p className="py-4 text-sm text-muted-foreground">
                        No active incoming shares left to map. Accept a share first, or every share already has a mapping.
                    </p>
                ) : (
                    <div className="max-h-80 space-y-1.5 overflow-y-auto">
                        {candidates.map((s) => (
                            <Button
                                key={s.id}
                                variant="outline"
                                className="h-auto w-full justify-start py-2 text-left"
                                disabled={create.isPending}
                                onClick={() => pick(s.id)}
                            >
                                <span className="flex flex-col items-start">
                                    <span className="text-sm">{s.name || `@${s.sender_username}:${s.sender_instance}`}</span>
                                    <span className="font-mono text-[11px] text-muted-foreground">@{s.sender_username}:{s.sender_instance}</span>
                                </span>
                            </Button>
                        ))}
                    </div>
                )}
            </DialogContent>
        </Dialog>
    )
}
