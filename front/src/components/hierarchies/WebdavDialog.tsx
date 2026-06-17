import {type ReactNode, useState} from 'react'
import {Check, Copy, Eye, EyeOff, Loader2, RefreshCw} from 'lucide-react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle, DialogTrigger,} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {useWebdav, useWebdavMutations} from '@/hooks/useHierarchies'
import {apiErrorMessage} from '@/api/client'

/** Mount-info popup: the WebDAV URL + token (hidden by default), a regenerate button and a use_redirect toggle. */
export function WebdavDialog({hierarchyId, trigger}: { hierarchyId: string; trigger: ReactNode }) {
    const [open, setOpen] = useState(false)
    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>{trigger}</DialogTrigger>
            <DialogContent>
                <DialogHeader>
                    <DialogTitle>WebDAV mount</DialogTitle>
                    <DialogDescription>
                        Mount this hierarchy as a network drive. Authenticate with HTTP Basic — your{' '}
                        <code className="text-xs">@user</code> as the username and the token below as the password.
                    </DialogDescription>
                </DialogHeader>
                {/* Mounts the body only when open, so the token isn't minted until the dialog is shown. */}
                {open && <WebdavBody hierarchyId={hierarchyId}/>}
            </DialogContent>
        </Dialog>
    )
}

function WebdavBody({hierarchyId}: { hierarchyId: string }) {
    const {data, isPending, isError, error} = useWebdav(hierarchyId)
    const {regenerate, setUseRedirect} = useWebdavMutations(hierarchyId)
    const [showToken, setShowToken] = useState(false)
    const [copied, setCopied] = useState<'url' | 'token' | null>(null)

    if (isPending) {
        return (
            <div className="flex items-center justify-center py-8 text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin"/>
            </div>
        )
    }
    if (isError || !data) {
        return <p className="py-4 text-sm text-muted-foreground">{apiErrorMessage(error)}</p>
    }

    const copy = (what: 'url' | 'token', value: string) => {
        void navigator.clipboard.writeText(value).then(
            () => {
                setCopied(what)
                setTimeout(() => setCopied((c) => (c === what ? null : c)), 1500)
            },
            () => toast.error('Could not copy to clipboard'),
        )
    }

    return (
        <div className="space-y-4">
            {!data.enabled && (
                <p className="rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">
                    This hierarchy is disabled — the mount will not respond until you enable it.
                </p>
            )}

            {/* Mount URL */}
            <div className="space-y-1.5">
                <Label htmlFor="webdav-url">Mount URL</Label>
                <div className="flex gap-2">
                    <Input id="webdav-url" readOnly value={data.url} className="font-mono text-xs"/>
                    <Button
                        variant="outline"
                        size="icon"
                        className="shrink-0"
                        onClick={() => copy('url', data.url)}
                        aria-label="Copy URL"
                        title="Copy URL"
                    >
                        {copied === 'url' ? <Check className="h-4 w-4"/> : <Copy className="h-4 w-4"/>}
                    </Button>
                </div>
            </div>

            {/* Token */}
            <div className="space-y-1.5">
                <Label htmlFor="webdav-token">Token (password)</Label>
                <div className="flex gap-2">
                    <Input
                        id="webdav-token"
                        readOnly
                        type={showToken ? 'text' : 'password'}
                        value={data.token}
                        className="font-mono text-xs"
                    />
                    <Button
                        variant="outline"
                        size="icon"
                        className="shrink-0"
                        onClick={() => setShowToken((s) => !s)}
                        aria-label={showToken ? 'Hide token' : 'Show token'}
                        title={showToken ? 'Hide token' : 'Show token'}
                    >
                        {showToken ? <EyeOff className="h-4 w-4"/> : <Eye className="h-4 w-4"/>}
                    </Button>
                    <Button
                        variant="outline"
                        size="icon"
                        className="shrink-0"
                        onClick={() => copy('token', data.token)}
                        aria-label="Copy token"
                        title="Copy token"
                    >
                        {copied === 'token' ? <Check className="h-4 w-4"/> : <Copy className="h-4 w-4"/>}
                    </Button>
                </div>
                <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 gap-1.5 text-xs text-muted-foreground"
                    disabled={regenerate.isPending}
                    onClick={() =>
                        regenerate.mutate(undefined, {
                            onSuccess: () => {
                                setShowToken(true)
                                toast.success('Token regenerated — re-mount any connected clients')
                            },
                            onError: (e) => toast.error(apiErrorMessage(e)),
                        })
                    }
                >
                    {regenerate.isPending ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin"/>
                    ) : (
                        <RefreshCw className="h-3.5 w-3.5"/>
                    )}
                    Regenerate token
                </Button>
            </div>

            {/* use_redirect toggle */}
            <div className="flex items-start justify-between gap-4 rounded-md border border-border px-3 py-2.5">
                <div className="space-y-0.5">
                    <Label htmlFor="webdav-redirect" className="text-sm">
                        Redirect reads
                    </Label>
                    <p className="text-xs text-muted-foreground">
                        When on, file reads 302-redirect to presigned URLs. When off, the backend proxies the bytes.
                    </p>
                </div>
                <Switch
                    id="webdav-redirect"
                    checked={data.use_redirect}
                    disabled={setUseRedirect.isPending}
                    onCheckedChange={(v) =>
                        setUseRedirect.mutate(v, {
                            onError: (e) => toast.error(apiErrorMessage(e)),
                        })
                    }
                />
            </div>
        </div>
    )
}
