import {useMemo} from 'react'
import {Copy, Link2, Share2, Users, X} from 'lucide-react'
import {toast} from 'sonner'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {Button} from '@/components/ui/button'
import {RevokePublicLinkButton, RevokeShareButton} from '@/components/shares/RevokeControls'
import {useOutgoingShares} from '@/hooks/useShares'
import {usePublicShares} from '@/hooks/usePublicShares'
import {publicShareUrl} from '@/api/publicShares'
import {outgoingEntry, ShareInfoPopover} from '@/components/shares/ShareInfoPopover'
import {PublicShareInfoPopover} from '@/components/shares/PublicShareInfoPopover'
import {ShareStatusBadge} from '@/components/shares/ShareStatusBadge'
import {useAuthStore} from '@/stores/auth'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {cn, TagPath} from '@/lib/utils'
import type {ShareResponse} from '@/lib/types'
import type {PublicShareSummary} from '@/api/publicShares'

/**
 * Steady-state share *browsing* lives on the tag tree (feature 34 §10); the Shares tabs stay for
 * what is actionable. Everything here is computed client-side by prefix from the share lists the
 * app already fetched — no extra queries.
 */
export interface TagShareInfo {
    /** Shares anchored exactly on this tag. */
    own: ShareResponse[]
    /** Public links anchored exactly on this tag. */
    links: PublicShareSummary[]
    /** Inherited from an ancestor — without this nobody realises that sharing `Era.2024` exposed
     *  `Era.2024.Vietnam`. */
    inherited: boolean
}

const isLive = (s: { status: string }) => s.status !== 'revoked' && s.status !== 'tombstoned'

/** Index every tag path's share state once, so a row lookup is O(1). */
export function useTagShareIndex(): (path: string) => TagShareInfo {
    const {data: outgoing} = useOutgoingShares()
    const {data: publics} = usePublicShares()

    return useMemo(() => {
        const shares = (outgoing ?? []).filter(isLive)
        const links = (publics ?? []).filter(isLive)
        const anchors = new Set<string>([...shares.map((s) => s.tag_path), ...links.map((l) => l.tag_path)])
        return (path: string): TagShareInfo => ({
            own: shares.filter((s) => s.tag_path === path),
            links: links.filter((l) => l.tag_path === path),
            inherited: [...anchors].some((a) => path.startsWith(`${a}.`)),
        })
    }, [outgoing, publics])
}

function initials(username: string): string {
    return username.slice(0, 2).toUpperCase()
}

/** The popover rows are one line tall, so the shared revoke controls get a bare `X` here. */
function CompactRevoke() {
    return (
        <button className="shrink-0 rounded p-0.5 text-muted-foreground hover:text-destructive" title="Revoke">
            <X className="h-3.5 w-3.5"/>
        </button>
    )
}

/**
 * The badge on a tag row: an avatar stack (≤3) or `Share2` + count for outgoing, `Link2` for public
 * links, and a fainter inherited marker for descendants of a shared tag. Clicking opens the
 * popover; the `…` menu keeps structural actions.
 */
export function TagShareBadge({
                                  path,
                                  info,
                                  onShare,
                              }: {
    path: string
    info: TagShareInfo
    onShare: (path: string) => void
}) {
    const username = useAuthStore((s) => s.user?.username ?? '')
    const domain = useAuthStore((s) => s.instance) || GLOBAL_DOMAIN
    const hasOwn = info.own.length > 0 || info.links.length > 0
    if (!hasOwn && !info.inherited) return null

    if (!hasOwn) {
        return (
            <span title="Shared through a parent tag" className="shrink-0 opacity-30">
                <Share2 className="h-3 w-3"/>
            </span>
        )
    }

    return (
        <Popover>
            <PopoverTrigger asChild>
                <button
                    onClick={(e) => e.stopPropagation()}
                    className="flex shrink-0 items-center gap-0.5 rounded px-1 text-[10px] text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="Sharing"
                >
                    {info.own.length > 0 &&
                        (info.own.length <= 3 ? (
                            <span className="flex -space-x-1">
                                {info.own.map((s) => (
                                    <span
                                        key={s.id}
                                        title={`${s.recipient_username}@${s.recipient_instance}`}
                                        className="flex h-4 w-4 items-center justify-center rounded-full bg-primary/15 text-[8px] font-medium text-primary ring-1 ring-background"
                                    >
                                        {initials(s.recipient_username)}
                                    </span>
                                ))}
                            </span>
                        ) : (
                            <>
                                <Share2 className="h-3 w-3"/>
                                {info.own.length}
                            </>
                        ))}
                    {info.links.length > 0 && <Link2 className="h-3 w-3"/>}
                </button>
            </PopoverTrigger>
            <PopoverContent align="start" className="w-80 p-2" onClick={(e) => e.stopPropagation()}>
                <p className="px-1 pb-1 text-[11px] text-muted-foreground">
                    Sharing <span className="font-mono">{TagPath.toDisplay(path)}</span>
                </p>

                {/* Each row's full detail reuses the shares tabs' own `(i)` popup as a sub-popover,
                    rather than cramming status, flags and timestamps onto the row. */}
                {info.own.map((s) => (
                    <div key={s.id} className="flex items-center gap-1 rounded px-1 py-1 text-xs hover:bg-muted">
                        <Users className="h-3.5 w-3.5 shrink-0 opacity-60"/>
                        <span className="min-w-0 flex-1 truncate">
                            {s.recipient_username}@{s.recipient_instance}
                        </span>
                        <ShareStatusBadge status={s.status}/>
                        <ShareInfoPopover entries={[outgoingEntry(s)]}/>
                        <RevokeShareButton share={s} trigger={<CompactRevoke/>}/>
                    </div>
                ))}

                {info.links.map((l) => (
                    <div key={l.id} className="flex items-center gap-1 rounded px-1 py-1 text-xs hover:bg-muted">
                        <Link2 className="h-3.5 w-3.5 shrink-0 opacity-60"/>
                        <span className="min-w-0 flex-1 truncate">{l.name}</span>
                        <PublicShareInfoPopover share={l}/>
                        <button
                            className="shrink-0 rounded p-0.5 text-muted-foreground hover:text-foreground"
                            title="Copy link"
                            onClick={() => {
                                void navigator.clipboard.writeText(publicShareUrl(domain, username, l.token))
                                toast.success('Link copied')
                            }}
                        >
                            <Copy className="h-3.5 w-3.5"/>
                        </button>
                        <RevokePublicLinkButton share={l} trigger={<CompactRevoke/>}/>
                    </div>
                ))}

                <Button
                    variant="ghost"
                    size="sm"
                    className={cn('mt-1 h-7 w-full justify-start text-xs')}
                    onClick={() => onShare(path)}
                >
                    <Share2 className="mr-1.5 h-3.5 w-3.5"/>
                    Share with someone else…
                </Button>
            </PopoverContent>
        </Popover>
    )
}
