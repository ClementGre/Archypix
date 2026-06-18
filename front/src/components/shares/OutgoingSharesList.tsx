import {useMemo} from 'react'
import {Ban, Loader2, Send} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Section} from '@/components/photos/detail/Section'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useOutgoingShares, useShareMutations} from '@/hooks/useShares'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import type {ShareResponse} from '@/lib/types'
import {ShareStatusBadge} from './ShareStatusBadge'
import {CreateShareDialog} from './CreateShareDialog'
import {type ShareInfoEntry, ShareInfoPopover, summarizeNames} from './ShareInfoPopover'

const REVOCABLE = new Set(['pending', 'pending_first_announcement', 'active', 'errored'])
const PENDING = new Set(['pending', 'pending_first_announcement'])
const CLOSED = new Set(['revoked', 'tombstoned'])

function RevokeButton({share, disabled, onRevoke}: { share: ShareResponse; disabled: boolean; onRevoke: () => void }) {
    if (!REVOCABLE.has(share.status)) return null
    return (
        <ConfirmDialog
            title="Revoke this share?"
            description={`Stop sharing with @${share.recipient_username}:${share.recipient_instance}. Their access and the shared pictures are removed immediately.`}
            confirmLabel="Revoke"
            destructive
            onConfirm={onRevoke}
            trigger={
                <Button
                    size="icon"
                    variant="ghost"
                    className="h-6 w-6 text-muted-foreground hover:text-destructive"
                    title="Revoke"
                    disabled={disabled}
                >
                    <Ban className="h-3.5 w-3.5"/>
                </Button>
            }
        />
    )
}

/** A tag and the shares (recipients) that target it, rendered as one card.
 *  Reused across the active / pending / closed sections. */
function GroupedShareRow({
                             tag,
                             shares,
                             revoking,
                             onFilterTag,
                             onRevoke,
                         }: {
    tag: string
    shares: ShareResponse[]
    revoking: boolean
    onFilterTag: (tag: string) => void
    onRevoke: (id: string) => void
}) {
    const nameLabel = summarizeNames(shares.map((s) => s.name))
    const entries: ShareInfoEntry[] = shares.map((s) => ({
        label: `→ @${s.recipient_username}:${s.recipient_instance}`,
        name: s.name,
        message: s.message,
    }))

    return (
        <div className="rounded-md border border-border px-2 py-1.5">
            <div className="flex items-center gap-1">
                <span className="min-w-0 flex-1 truncate text-xs font-medium" title={nameLabel}>
                    {nameLabel}
                </span>
                <span className="shrink-0 text-[11px] text-muted-foreground">{shares.length}</span>
                <ShareInfoPopover entries={entries}/>
            </div>

            <button
                onClick={() => onFilterTag(tag)}
                className="block max-w-full truncate text-left text-[11px] text-muted-foreground hover:text-primary"
                title="Filter to this tag"
            >
                {TagPath.toDisplay(tag)}
            </button>

            <div className="mt-1.5 space-y-1">
                {shares.map((share) => (
                    <div key={share.id} className="flex items-center justify-between gap-2">
                        <span className="min-w-0 truncate text-[11px] text-muted-foreground">
                            → @{share.recipient_username}:{share.recipient_instance}
                        </span>
                        <div className="flex shrink-0 items-center gap-1">
                            <ShareStatusBadge status={share.status}/>
                            <RevokeButton share={share} disabled={revoking} onRevoke={() => onRevoke(share.id)}/>
                        </div>
                    </div>
                ))}
            </div>
        </div>
    )
}

/** Group a flat list of shares by tag path, sorted by tag. */
function groupByTag(shares: ShareResponse[]): Array<[string, ShareResponse[]]> {
    const map = new Map<string, ShareResponse[]>()
    for (const s of shares) {
        const list = map.get(s.tag_path) ?? []
        list.push(s)
        map.set(s.tag_path, list)
    }
    return [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))
}

export function OutgoingSharesList() {
    const {data: shares, isPending, isError, error} = useOutgoingShares()
    const {revoke} = useShareMutations()
    const {update} = useGalleryParams()

    const onRevoke = (id: string) => revoke.mutate(id, {onError: (e) => toast.error(apiErrorMessage(e))})
    const onFilterTag = (tag: string) => update({tag})

    const {closed, pending, active} = useMemo(() => {
        const all = shares ?? []
        return {
            closed: groupByTag(all.filter((s) => CLOSED.has(s.status))),
            pending: groupByTag(all.filter((s) => PENDING.has(s.status))),
            active: groupByTag(all.filter((s) => !PENDING.has(s.status) && !CLOSED.has(s.status))),
        }
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

    const renderGroups = (groups: Array<[string, ShareResponse[]]>) =>
        groups.map(([tag, group]) => (
            <GroupedShareRow
                key={tag}
                tag={tag}
                shares={group}
                revoking={revoke.isPending}
                onFilterTag={onFilterTag}
                onRevoke={onRevoke}
            />
        ))

    const closedCount = closed.reduce((n, [, g]) => n + g.length, 0)
    const pendingCount = pending.reduce((n, [, g]) => n + g.length, 0)
    const activeCount = active.reduce((n, [, g]) => n + g.length, 0)

    return (
        <>
            {header}
            <div className="p-2">
                {closed.length > 0 && (
                    <Section id="outgoing-closed" title="Closed" count={closedCount} defaultOpen={false}>
                        <div className="space-y-1.5">{renderGroups(closed)}</div>
                    </Section>
                )}

                {pending.length > 0 && (
                    <Section id="outgoing-pending" title="Pending" count={pendingCount}>
                        <div className="space-y-1.5">{renderGroups(pending)}</div>
                    </Section>
                )}

                {active.length > 0 && (
                    <Section id="outgoing-active" title="Active" count={activeCount}>
                        <div className="space-y-1.5">{renderGroups(active)}</div>
                    </Section>
                )}
            </div>
        </>
    )
}
