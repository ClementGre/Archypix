import {useState} from 'react'
import {Ban, Check, Copy, Loader2, Pencil, Trash2} from 'lucide-react'
import {toast} from 'sonner'
import type {PublicShareSummary} from '@/api/publicShares'
import {publicShareUrl} from '@/api/publicShares'
import {apiErrorMessage} from '@/api/client'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {TagPath} from '@/lib/utils'
import {useAuthStore} from '@/stores/auth'
import {usePublicShareMutations} from '@/hooks/usePublicShares'
import {Button} from '@/components/ui/button'
import {Switch} from '@/components/ui/switch'
import {Section} from '@/components/photos/detail/Section'
import {Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle} from '@/components/ui/dialog'
import {PublicShareDialog} from './PublicShareDialog'
import {PublicShareInfoPopover} from './PublicShareInfoPopover'

type PublicLinksManagerProps = {
    shares?: PublicShareSummary[]
    isPending: boolean
}

export function PublicLinksManager({shares, isPending}: PublicLinksManagerProps) {
    const active = (shares ?? []).filter((s) => s.status === 'active')

    return (
        <Section id="public_links" title={`Public share links`} count={active.length} defaultOpen={false}>
            <div className="space-y-2 px-1 py-1">
                {isPending ? (
                    <div className="flex justify-center py-3">
                        <Loader2 className="h-4 w-4 animate-spin text-muted-foreground"/>
                    </div>
                ) : active.length === 0 ? (
                    <p className="py-2 text-xs text-muted-foreground">No public share links yet.</p>
                ) : (
                    <ul className="space-y-1.5">
                        {active.map((s) => (
                            <PublicLinkRow key={s.id} share={s}/>
                        ))}
                    </ul>
                )}
            </div>
        </Section>
    )
}

function PublicLinkRow({share}: { share: PublicShareSummary }) {
    const username = useAuthStore((s) => s.user?.username ?? '')
    const domain = useAuthStore((s) => s.instance) || GLOBAL_DOMAIN
    const [copied, setCopied] = useState(false)
    const [revokeOpen, setRevokeOpen] = useState(false)
    const [editOpen, setEditOpen] = useState(false)

    const url = publicShareUrl(domain, username, share.token)
    const copy = async () => {
        try {
            await navigator.clipboard.writeText(url)
            setCopied(true)
            setTimeout(() => setCopied(false), 1500)
        } catch {
            toast.error('Could not copy the link.')
        }
    }

    return (
        <li className="rounded-md border border-border p-2 text-sm">
            <div className="flex items-center gap-1">
                <div className="min-w-0 flex-1">
                    <div className="truncate font-medium">{share.name}</div>
                    <div className="truncate text-xs text-muted-foreground">{TagPath.toDisplay(share.tag_path)}</div>
                </div>
                <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 text-muted-foreground hover:text-destructive"
                    title="Revoke"
                    onClick={() => setRevokeOpen(true)}
                >
                    <Ban className="h-3.5 w-3.5"/>
                </Button>
                <Button size="icon" variant="ghost" className="h-7 w-7" title="Edit" onClick={() => setEditOpen(true)}>
                    <Pencil className="h-3.5 w-3.5"/>
                </Button>
                <Button size="icon" variant="ghost" className="h-7 w-7" title="Copy link" onClick={copy}>
                    {copied ? <Check className="h-3.5 w-3.5 text-emerald-500"/> : <Copy className="h-3.5 w-3.5"/>}
                </Button>
                <PublicShareInfoPopover share={share}/>
            </div>
            <div className="mt-1 flex flex-wrap gap-2 text-[11px] text-muted-foreground">
                <Flag on={share.permissions.allow_originals} label="originals"/>
                <Flag on={share.permissions.allow_upload} label="uploads"/>
                {share.has_password && <span className="rounded bg-muted px-1.5 py-0.5">🔒 password</span>}
                {share.derived_share_count > 0 && (
                    <span className="rounded bg-muted px-1.5 py-0.5">{share.derived_share_count} derived shares</span>
                )}
                {share.contribution_count > 0 && (
                    <span className="rounded bg-muted px-1.5 py-0.5">{share.contribution_count} contributed</span>
                )}
            </div>
            <PublicShareDialog share={share} open={editOpen} onOpenChange={setEditOpen} showTrigger={false}/>
            <RevokeDialog share={share} open={revokeOpen} onOpenChange={setRevokeOpen}/>
        </li>
    )
}

function Flag({on, label}: { on: boolean; label: string }) {
    return (
        <span
            className={on ? 'rounded bg-emerald-500/15 px-1.5 py-0.5 text-emerald-600 dark:text-emerald-400' : 'rounded px-1.5 py-0.5 bg-red-500/15 text-red-600 dark:text-red-400'}>
            {label}
        </span>
    )
}

function RevokeDialog({
                          share,
                          open,
                          onOpenChange,
                      }: {
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
