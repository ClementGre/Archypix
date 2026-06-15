import {Check, Images, Inbox, Loader2, X} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
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

/** A share carries a single local-tag mapping: show it, or offer to add one. */
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
      <div className="mt-2 border-t border-border pt-2">
        <p className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Local tag</p>
        {mapping ? (
            <Badge
                variant="secondary"
                className={cn('gap-1 font-normal', mapping.is_broken && 'line-through opacity-60')}
            >
              {TagPath.toDisplay(mapping.assign_tag)}
              <ConfirmDialog
                  title="Remove mapping?"
                  description="This local tag will be removed from the pictures received through this share."
                  confirmLabel="Remove"
                  destructive
                  onConfirm={() => onRemove(mapping.serviceId, mapping.ruleId)}
                  trigger={
                    <button aria-label="Remove mapping" disabled={busy} className="ml-0.5">
                      <X className="h-3 w-3"/>
                    </button>
                  }
              />
            </Badge>
        ) : (
            <TagPicker onSelect={(wire) => onAdd(shareId, wire)} triggerLabel="Map tag"/>
        )}
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

  return (
      <div className="space-y-2 p-2">
        {shares.map((share) => {
          const rejectable = share.status === 'pending' || share.status === 'active'
          return (
              <div
                  key={share.id}
                  className={cn('rounded-md border border-border p-2.5', params.share === share.id && 'ring-2 ring-primary')}
              >
                <div className="flex items-center justify-between gap-2">
              <span className="truncate text-sm font-medium">
                @{share.sender_username}
                <span className="text-muted-foreground">:{share.sender_instance}</span>
              </span>
                  <ShareStatusBadge status={share.status}/>
                </div>

                <div className="mt-2 flex flex-wrap gap-1.5">
                  {share.status === 'pending' && (
                      <Button
                          size="sm"
                          className="h-7 gap-1 px-2"
                          disabled={accept.isPending}
                          onClick={() => accept.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})}
                      >
                        <Check className="h-3.5 w-3.5"/> Accept
                      </Button>
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
                  {rejectable && (
                      <ConfirmDialog
                          title="Reject this share?"
                          description="Pictures received through this share will be removed from your library."
                          confirmLabel="Reject"
                          destructive
                          onConfirm={() => reject.mutate(share.id, {onError: (e) => toast.error(apiErrorMessage(e))})}
                          trigger={
                            <Button size="sm" variant="outline" className="h-7 gap-1 px-2" disabled={reject.isPending}>
                              <X className="h-3.5 w-3.5"/> Reject
                            </Button>
                          }
                      />
                  )}
                </div>

                {share.status === 'active' && (
                    <MappingControl
                        shareId={share.id}
                        mapping={forShare(share.id)[0]}
                        busy={isBusy}
                        onAdd={onAddMapping}
                        onRemove={onRemoveMapping}
                    />
                )}
              </div>
          )
        })}
      </div>
  )
}
