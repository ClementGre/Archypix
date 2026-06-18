import {type ReactNode, useMemo, useState} from 'react'
import {Check, Images, Inbox, Loader2, Share2, X} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Section} from '@/components/photos/detail/Section'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useIncomingShares, useOutgoingShares, useShareMutations} from '@/hooks/useShares'
import {type ShareMapping, useShareMappings} from '@/hooks/useShareMappings'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {TagPicker} from '@/components/tags/TagPicker'
import {cn, TagPath} from '@/lib/utils'
import type {IncomingShareResponse, ShareResponse} from '@/lib/types'
import {ShareInfoPopover} from './ShareInfoPopover'
import {CreateShareDialog} from './CreateShareDialog'

const PENDING = new Set(['pending'])
const CLOSED = new Set(['revoked', 'tombstoned'])

function sharedToMeTag(share: IncomingShareResponse): string {
    return (
        share.shared_tag_path ??
        `SharedToMe.${`${share.sender_username}@${share.sender_instance}`.replace(/@/g, '_AT_').replace(/\./g, '_DOT_')}`
    )
}

/** A share carries a single local-tag mapping: show it inline, or offer to add one. */
function MappingControl({
                          shareId,
                          mapping,
                          busy,
                          onAdd,
                          onRemove,
                        }: {
  shareId: string
  mapping: ShareMapping | undefined
  busy: boolean
  onAdd: (shareId: string, wire: string) => void
  onRemove: (serviceId: string, ruleId: string) => void
}) {
  return (
      <div className="mt-1.5 flex items-center gap-1.5">
          <span className="shrink-0 text-[11px] text-muted-foreground">Tag</span>
        {mapping ? (
            <Badge
                variant="secondary"
                className={cn('gap-1 font-normal', mapping.is_broken && 'line-through opacity-60')}
            >
                <span className="truncate">{TagPath.toDisplay(mapping.assign_tag)}</span>
              <ConfirmDialog
                  title="Remove mapping?"
                  description="This local tag will be removed from the pictures received through this share."
                  confirmLabel="Remove"
                  destructive
                  onConfirm={() => onRemove(mapping.serviceId, mapping.ruleId)}
                  trigger={
                      <button aria-label="Remove mapping" disabled={busy} className="ml-0.5 shrink-0">
                      <X className="h-3 w-3"/>
                    </button>
                  }
              />
            </Badge>
        ) : (
            <TagPicker
                onSelect={(wire) => onAdd(shareId, wire)}
                trigger={
                    <Button variant="outline" size="sm" className="h-6 gap-1 px-1.5 text-[11px]">
                        Map tag
                    </Button>
                }
            />
        )}
      </div>
  )
}

function ShareRow({
                      share,
                      highlighted,
                      accepting,
                      rejecting,
                      sharebackOfLabel,
                      onAccept,
                      onReject,
                      onView,
                      onShareBack,
                      mapping,
                  }: {
    share: IncomingShareResponse
    highlighted: boolean
    accepting: boolean
    rejecting: boolean
    sharebackOfLabel: string | null
    onAccept: () => void
    onReject: () => void
    onView: () => void
    onShareBack: () => void
    mapping?: ReactNode
}) {
    const rejectable = share.status === 'pending' || share.status === 'active'
    return (
        <div className={cn('rounded-md border border-border px-2 py-1.5', highlighted && 'ring-2 ring-primary')}>
            <div className="flex items-center justify-between gap-2">
          <span className="truncate text-xs font-medium">
            @{share.sender_username}
              <span className="text-muted-foreground">:{share.sender_instance}</span>
          </span>
                <div className="flex shrink-0 items-center gap-1">
                    <ShareInfoPopover
                        entries={[{
                            name: share.name,
                            message: share.message,
                            status: share.status,
                            allowShareBack: share.allow_share_back,
                            future: share.future,
                            sharedTag: share.shared_tag_path,
                            createdAt: share.created_at,
                            lastReceivedAt: share.last_announcement_received_at,
                            closedAt: share.revoked_at,
                            sharebackOf: sharebackOfLabel,
                        }]}
                        footer={
                            share.allow_share_back && share.status === 'active' ? (
                                <Button
                                    size="sm"
                                    variant="outline"
                                    className="h-7 w-full gap-1.5 text-xs"
                                    onClick={onShareBack}
                                >
                                    <Share2 className="h-3.5 w-3.5"/> Share back
                                </Button>
                            ) : undefined
                        }
                    />
                    {share.status === 'active' && (
                        <Button
                            size="icon"
                            variant="ghost"
                            className="h-6 w-6 text-muted-foreground hover:text-foreground"
                            title="View photos"
                            onClick={onView}
                        >
                            <Images className="h-3.5 w-3.5"/>
                        </Button>
                    )}
                    {rejectable && (
                        <ConfirmDialog
                            title="Reject this share?"
                            description="Pictures received through this share will be removed from your library."
                            confirmLabel="Reject"
                            destructive
                            onConfirm={onReject}
                            trigger={
                                <Button
                                    size="icon"
                                    variant="ghost"
                                    className="h-6 w-6 text-muted-foreground hover:text-destructive"
                                    title="Reject"
                                    disabled={rejecting}
                                >
                                    <X className="h-3.5 w-3.5"/>
                                </Button>
                            }
                        />
                    )}
                </div>
            </div>

            <p className="mt-0.5 truncate text-[11px] text-muted-foreground" title={share.name}>
                {share.name}
            </p>

            {share.status === 'pending' && (
                <div className="mt-1.5">
                    <Button size="sm" className="h-6 gap-1 px-2 text-[11px]" disabled={accepting} onClick={onAccept}>
                        <Check className="h-3.5 w-3.5"/> Accept
                    </Button>
                </div>
            )}

            {share.status === 'active' && mapping}
      </div>
  )
}

export function IncomingSharesList() {
  const {data: shares, isPending, isError, error} = useIncomingShares()
    const {data: outgoing} = useOutgoingShares()
  const {accept, reject} = useShareMutations()
  const {update, params} = useGalleryParams()
  const {forShare, addMapping, removeMapping, isBusy} = useShareMappings()
    const [sharebackTarget, setSharebackTarget] = useState<IncomingShareResponse | null>(null)

    // incoming.shareback_of references one of the user's own outgoing shares.
    const outgoingById = useMemo(() => {
        const m = new Map<string, ShareResponse>()
        for (const o of outgoing ?? []) m.set(o.id, o)
        return m
    }, [outgoing])
    const sharebackLabel = (share: IncomingShareResponse): string | null => {
        if (!share.shareback_of) return null
        const o = outgoingById.get(share.shareback_of)
        return o ? `${o.name} → @${o.recipient_username}:${o.recipient_instance}` : 'your share'
    }

  const onAddMapping = (shareId: string, wire: string) => {
    addMapping(shareId, wire).catch((e) => toast.error('Could not map tag', {description: apiErrorMessage(e)}))
  }
  const onRemoveMapping = (serviceId: string, ruleId: string) => {
    removeMapping(serviceId, ruleId).catch((e) =>
        toast.error('Could not remove mapping', {description: apiErrorMessage(e)}),
    )
  }

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

    const closed = shares.filter((s) => CLOSED.has(s.status))
    const pending = shares.filter((s) => PENDING.has(s.status))
    const active = shares.filter((s) => !PENDING.has(s.status) && !CLOSED.has(s.status))

    const renderRow = (share: IncomingShareResponse) => (
        <ShareRow
            key={share.id}
            share={share}
            highlighted={params.share === share.id}
            accepting={accept.isPending}
            rejecting={reject.isPending}
            sharebackOfLabel={sharebackLabel(share)}
            onAccept={() => accept.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})}
            onReject={() => reject.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})}
            onView={() => update({tag: sharedToMeTag(share), scope: 'all'})}
            onShareBack={() => setSharebackTarget(share)}
            mapping={
                <MappingControl
                    shareId={share.id}
                    mapping={forShare(share.id)[0]}
                    busy={isBusy}
                    onAdd={onAddMapping}
                    onRemove={onRemoveMapping}
                />
            }
        />
    )

    return (
        <div className="p-2">
            {closed.length > 0 && (
                <Section id="incoming-closed" title="Canceled" count={closed.length} defaultOpen={false}>
                    <div className="space-y-1.5">{closed.map(renderRow)}</div>
                </Section>
            )}
            {pending.length > 0 && (
                <Section id="incoming-pending" title="Pending" count={pending.length}>
                    <div className="space-y-1.5">{pending.map(renderRow)}</div>
                </Section>
            )}
            {active.length > 0 && (
                <Section id="incoming-active" title="Active" count={active.length}>
                    <div className="space-y-1.5">{active.map(renderRow)}</div>
                </Section>
            )}

            <CreateShareDialog
                open={!!sharebackTarget}
                onOpenChange={(o) => {
                    if (!o) setSharebackTarget(null)
                }}
                showTrigger={false}
                initialShareback={sharebackTarget}
                initialTag={sharebackTarget ? forShare(sharebackTarget.id)[0]?.assign_tag : undefined}
            />
      </div>
  )
}
