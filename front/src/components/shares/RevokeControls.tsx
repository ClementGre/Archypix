// Revoking a share is the one destructive action reachable from three places — the Shares tabs, the
// public-links manager and the tag tree's share popover (34 §10). The confirmation, the wording and
// the mutation live here once, so a new surface gets the same gate for free.

import {type ReactNode, useState} from 'react'
import {Ban, Trash2} from 'lucide-react'
import {toast} from 'sonner'
import {apiErrorMessage} from '@/api/client'
import {useShareMutations} from '@/hooks/useShares'
import {usePublicShareMutations} from '@/hooks/usePublicShares'
import {Button} from '@/components/ui/button'
import {Switch} from '@/components/ui/switch'
import {Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import type {PublicShareSummary} from '@/api/publicShares'
import type {ShareResponse} from '@/lib/types'

const REVOCABLE = new Set(['pending', 'pending_first_announcement', 'active', 'errored'])

/** Whether a share is still in a state that can be revoked. */
export const isRevocable = (share: ShareResponse): boolean => REVOCABLE.has(share.status)

/** Revoke one outgoing share, behind a confirm. Renders nothing once the share is closed. */
export function RevokeShareButton({share, disabled, trigger}: {
    share: ShareResponse
    disabled?: boolean
    /** Override the default icon button (e.g. a bare `X` in a compact popover row). */
    trigger?: ReactNode
}) {
    const {revoke} = useShareMutations()
    if (!isRevocable(share)) return null

    return (
        <ConfirmDialog
            title="Revoke this share?"
            description={`Stop sharing with @${share.recipient_username}:${share.recipient_instance}. Their access and the shared pictures are removed immediately.`}
            confirmLabel="Revoke"
            destructive
            onConfirm={() =>
                revoke.mutate(share.id, {
                    onSuccess: () => toast.success('Share revoked'),
                    onError: (e) => toast.error(apiErrorMessage(e)),
                })
            }
            trigger={
                trigger ?? (
                    <Button
                        size="icon"
                        variant="ghost"
                        className="h-6 w-6 text-muted-foreground hover:text-destructive"
                        title="Revoke"
                        disabled={disabled}
                    >
                        <Ban className="h-3.5 w-3.5"/>
                    </Button>
                )
            }
        />
    )
}

/**
 * Revoking a public link is more than a yes/no: derived private shares and anonymous contributions
 * each get their own opt-in, so it is a real dialog rather than a `ConfirmDialog`.
 */
export function RevokePublicLinkDialog({share, open, onOpenChange}: {
    share: PublicShareSummary
    open: boolean
    onOpenChange: (v: boolean) => void
}) {
    const {revoke} = usePublicShareMutations()
    const [cascade, setCascade] = useState(false)
    const [trash, setTrash] = useState(false)

    const submit = async () => {
        try {
            await revoke.mutateAsync({id: share.id, cascade, trash})
            toast.success('Public share link revoked.')
            onOpenChange(false)
        } catch (e) {
            toast.error(apiErrorMessage(e))
        }
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="max-w-md">
                <DialogHeader>
                    <DialogTitle>Revoke "{share.name}"?</DialogTitle>
                </DialogHeader>
                <p className="text-sm text-muted-foreground">
                    The link stops working immediately. This does not delete the pictures.
                </p>
                {share.derived_share_count > 0 && (
                    <label className="flex items-center justify-between gap-3 text-sm">
                        <span>Also revoke the {share.derived_share_count} derived private share(s)</span>
                        <Switch checked={cascade} onCheckedChange={setCascade}/>
                    </label>
                )}
                {share.contribution_count > 0 && (
                    <label className="flex items-center justify-between gap-3 text-sm">
                        <span className="inline-flex items-center gap-1.5">
                            <Trash2 className="h-4 w-4"/> Move the {share.contribution_count} contribution(s) to trash
                        </span>
                        <Switch checked={trash} onCheckedChange={setTrash}/>
                    </label>
                )}
                <DialogFooter>
                    <Button variant="ghost" onClick={() => onOpenChange(false)}>
                        Cancel
                    </Button>
                    <Button variant="destructive" onClick={submit} disabled={revoke.isPending}>
                        Revoke
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}

/** The dialog above with its own trigger, for surfaces that have no open-state of their own. */
export function RevokePublicLinkButton({share, trigger}: { share: PublicShareSummary; trigger: ReactNode }) {
    const [open, setOpen] = useState(false)
    return (
        <>
            <span onClick={() => setOpen(true)}>{trigger}</span>
            <RevokePublicLinkDialog share={share} open={open} onOpenChange={setOpen}/>
        </>
    )
}
