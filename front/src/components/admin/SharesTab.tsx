import {RefreshCw} from 'lucide-react'
import {toast} from 'sonner'
import {Table, TableBody, TableCell, TableHead, TableHeader, TableRow} from '@/components/ui/table'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Skeleton} from '@/components/ui/skeleton'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useAdminShareMutations, useErroredShares, useFederationInstances} from '@/hooks/useAdmin'
import {apiErrorMessage} from '@/api/client'
import type {ErroredShareResponse, FederationInstanceResponse} from '@/lib/types'

function formatRelative(iso: string | null): string {
    if (!iso) return '—'
    const diff = Date.now() - new Date(iso).getTime()
    const mins = Math.floor(diff / 60_000)
    if (mins < 1) return 'just now'
    if (mins < 60) return `${mins}m ago`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}h ago`
    return `${Math.floor(hrs / 24)}d ago`
}

function formatFuture(iso: string | null): string {
    if (!iso) return '—'
    const diff = new Date(iso).getTime() - Date.now()
    if (diff <= 0) return 'now'
    const mins = Math.ceil(diff / 60_000)
    if (mins < 60) return `in ${mins}m`
    return `in ${Math.ceil(mins / 60)}h`
}

function ErroredShareRow({share}: { share: ErroredShareResponse }) {
    const {forceReconcile} = useAdminShareMutations()

    const handleReconcile = async () => {
        try {
            await forceReconcile.mutateAsync(share.id)
            toast.success('Reconcile triggered')
        } catch (e) {
            toast.error('Failed to trigger reconcile', {description: apiErrorMessage(e)})
        }
    }

    return (
        <TableRow>
            <TableCell className="font-mono text-xs">{share.owner_username}</TableCell>
            <TableCell className="font-mono text-xs max-w-xs truncate" title={share.tag_path}>
                {share.tag_path}
            </TableCell>
            <TableCell className="font-mono text-xs">
                {share.recipient_username}@{share.recipient_instance}
            </TableCell>
            <TableCell className="text-sm text-muted-foreground">
                {formatRelative(share.last_error_at)}
            </TableCell>
            <TableCell className="text-sm text-muted-foreground">
                {formatFuture(share.next_retry_at)}
            </TableCell>
            <TableCell>
                <ConfirmDialog
                    trigger={
                        <Button variant="outline" size="sm" className="gap-1.5">
                            <RefreshCw className="h-3.5 w-3.5"/>
                            Force reconcile
                        </Button>
                    }
                    title="Force reconcile?"
                    description="Clears the retry backoff and immediately wakes the owner's pipeline to retry delivery."
                    confirmLabel="Reconcile"
                    onConfirm={handleReconcile}
                />
            </TableCell>
        </TableRow>
    )
}

function ErroredSharesTable() {
    const {data, isLoading} = useErroredShares()

    return (
        <div className="space-y-3">
            <div className="flex items-center justify-between">
                <h2 className="text-sm font-medium">Errored outgoing shares</h2>
                {!isLoading && data && (
                    <p className="text-xs text-muted-foreground">{data.length} errored</p>
                )}
            </div>
            <div className="rounded-md border">
                <Table>
                    <TableHeader>
                        <TableRow>
                            <TableHead>Owner</TableHead>
                            <TableHead>Tag</TableHead>
                            <TableHead>Recipient</TableHead>
                            <TableHead>Last error</TableHead>
                            <TableHead>Next retry</TableHead>
                            <TableHead/>
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {isLoading ? (
                            Array.from({length: 3}).map((_, i) => (
                                <TableRow key={i}>
                                    {Array.from({length: 6}).map((_, j) => (
                                        <TableCell key={j}><Skeleton className="h-4 w-full"/></TableCell>
                                    ))}
                                </TableRow>
                            ))
                        ) : data?.length === 0 ? (
                            <TableRow>
                                <TableCell colSpan={6} className="text-center text-muted-foreground py-8">
                                    No errored shares
                                </TableCell>
                            </TableRow>
                        ) : (
                            data?.map((share) => (
                                <ErroredShareRow key={share.id} share={share}/>
                            ))
                        )}
                    </TableBody>
                </Table>
            </div>
        </div>
    )
}

function FederationInstanceRow({instance}: { instance: FederationInstanceResponse }) {
    return (
        <TableRow>
            <TableCell className="font-mono text-sm">{instance.instance}</TableCell>
            <TableCell className="text-sm">{instance.outgoing_share_count}</TableCell>
            <TableCell className="text-sm">{instance.incoming_share_count}</TableCell>
            <TableCell>
                {instance.errored_share_count > 0 ? (
                    <Badge variant="secondary" className="bg-red-500/15 text-red-500 border-0">
                        {instance.errored_share_count} errored
                    </Badge>
                ) : (
                    <Badge variant="secondary" className="bg-emerald-500/15 text-emerald-500 border-0">
                        OK
                    </Badge>
                )}
            </TableCell>
        </TableRow>
    )
}

function FederationTable() {
    const {data, isLoading} = useFederationInstances()

    return (
        <div className="space-y-3">
            <h2 className="text-sm font-medium">Federated instances</h2>
            <div className="rounded-md border">
                <Table>
                    <TableHeader>
                        <TableRow>
                            <TableHead>Instance</TableHead>
                            <TableHead>Outgoing shares</TableHead>
                            <TableHead>Incoming shares</TableHead>
                            <TableHead>Status</TableHead>
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {isLoading ? (
                            Array.from({length: 3}).map((_, i) => (
                                <TableRow key={i}>
                                    {Array.from({length: 4}).map((_, j) => (
                                        <TableCell key={j}><Skeleton className="h-4 w-full"/></TableCell>
                                    ))}
                                </TableRow>
                            ))
                        ) : data?.length === 0 ? (
                            <TableRow>
                                <TableCell colSpan={4} className="text-center text-muted-foreground py-8">
                                    No federated instances
                                </TableCell>
                            </TableRow>
                        ) : (
                            data?.map((inst) => (
                                <FederationInstanceRow key={inst.instance} instance={inst}/>
                            ))
                        )}
                    </TableBody>
                </Table>
            </div>
        </div>
    )
}

export function SharesTab() {
    return (
        <div className="space-y-8">
            <ErroredSharesTable/>
            <FederationTable/>
        </div>
    )
}
