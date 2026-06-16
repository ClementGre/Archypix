import type {ReactNode} from 'react'
import {Check, Images, Inbox, Loader2, X} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Section} from '@/components/photos/detail/Section'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useIncomingShares, useShareMutations} from '@/hooks/useShares'
import {type ShareMapping, useShareMappings} from '@/hooks/useShareMappings'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {TagPicker} from '@/components/tags/TagPicker'
import {cn, TagPath} from '@/lib/utils'
import type {IncomingShareResponse} from '@/lib/types'
import {ShareStatusBadge} from './ShareStatusBadge'

function sharedToMeTag(share: IncomingShareResponse): string {
  const label = `${share.sender_username}@${share.sender_instance}`.replace(/@/g, '_AT_').replace(/\./g, '_DOT_')
  return `SharedToMe.${label}`
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
                      onAccept,
                      onReject,
                      onView,
                      mapping,
                  }: {
    share: IncomingShareResponse
    highlighted: boolean
    accepting: boolean
    rejecting: boolean
    onAccept: () => void
    onReject: () => void
    onView: () => void
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
                    <ShareStatusBadge status={share.status}/>
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
  const {accept, reject} = useShareMutations()
  const {update, params} = useGalleryParams()
  const {forShare, addMapping, removeMapping, isBusy} = useShareMappings()

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

    const pending = shares.filter((s) => s.status === 'pending')
    const rest = shares.filter((s) => s.status !== 'pending')

    const renderRow = (share: IncomingShareResponse) => (
        <ShareRow
            key={share.id}
            share={share}
            highlighted={params.share === share.id}
            accepting={accept.isPending}
            rejecting={reject.isPending}
            onAccept={() => accept.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})}
            onReject={() => reject.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})}
            onView={() => update({tag: sharedToMeTag(share), scope: 'all'})}
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
            {pending.length > 0 && (
                <Section id="incoming-pending" title="Pending" count={pending.length}>
                    <div className="space-y-1.5">{pending.map(renderRow)}</div>
                </Section>
            )}
            <div className="space-y-1.5 pt-2">{rest.map(renderRow)}</div>
      </div>
  )
}
