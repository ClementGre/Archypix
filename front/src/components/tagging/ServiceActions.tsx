import {useState} from 'react'
import {toast} from 'sonner'
import {MoreHorizontal, Power, PowerOff, Trash2} from 'lucide-react'
import {
    AlertDialog,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import {Button} from '@/components/ui/button'
import {Checkbox} from '@/components/ui/checkbox'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger} from '@/components/ui/dropdown-menu'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {ServiceBase} from '@/lib/types'

type Lifecycle = 'disable' | 'delete'

/**
 * Enable / disable / delete menu for a service. Disabling and deleting both ask what happens to
 * the service's tags; promotion defaults on when pruning removed their manual copies (39 §4.7).
 */
export function ServiceActions({service, onDeleted}: { service: ServiceBase; onDeleted?: () => void }) {
    const {update, remove} = useTaggingMutations()
    const [dialog, setDialog] = useState<Lifecycle | null>(null)
    const [promote, setPromote] = useState(false)
    const onError = (err: unknown) => toast.error(apiErrorMessage(err))

    const open = (kind: Lifecycle) => {
        setPromote(kind === 'delete' || service.prune_shadowed_manual_tags)
        setDialog(kind)
    }
    const confirm = () => {
        if (dialog === 'disable') {
            update.mutate({id: service.id, body: {enabled: false, promote_tags: promote}}, {onError})
        } else {
            remove.mutate({id: service.id, promoteTags: promote}, {onSuccess: onDeleted, onError})
        }
        setDialog(null)
    }
    const pending = update.isPending || remove.isPending

    return (
        <span onClick={(e) => e.stopPropagation()}>
            <DropdownMenu>
                <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="icon" className="h-7 w-7 text-muted-foreground" aria-label="Service actions">
                        <MoreHorizontal className="h-4 w-4"/>
                    </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                    {service.enabled ? (
                        <DropdownMenuItem onSelect={() => open('disable')} disabled={pending}>
                            <PowerOff className="mr-2 h-4 w-4"/> Disable…
                        </DropdownMenuItem>
                    ) : (
                        <DropdownMenuItem
                            onSelect={() => update.mutate({id: service.id, body: {enabled: true}}, {onError})}
                            disabled={pending}
                        >
                            <Power className="mr-2 h-4 w-4"/> Enable
                        </DropdownMenuItem>
                    )}
                    <DropdownMenuItem onSelect={() => open('delete')} disabled={pending} className="text-destructive">
                        <Trash2 className="mr-2 h-4 w-4"/> Delete…
                    </DropdownMenuItem>
                </DropdownMenuContent>
            </DropdownMenu>

            <AlertDialog open={dialog !== null} onOpenChange={(o) => !o && setDialog(null)}>
                <AlertDialogContent>
                    <AlertDialogHeader>
                        <AlertDialogTitle>{dialog === 'disable' ? 'Disable service' : 'Delete service'}</AlertDialogTitle>
                        <AlertDialogDescription>
                            {dialog === 'disable'
                                ? 'Its tags leave every picture until you re-enable it, which re-derives them.'
                                : 'Its tags leave every picture for good.'}{' '}
                            Pictures shared only through these tags are unshared meanwhile.
                        </AlertDialogDescription>
                    </AlertDialogHeader>
                    <label className="flex items-start gap-2 text-sm">
                        <Checkbox checked={promote} onCheckedChange={(v) => setPromote(v === true)} className="mt-0.5"/>
                        <span>
                            Promote tags to manual
                            <span className="block text-xs text-muted-foreground">
                                Keep them as your own tags instead.
                                {service.prune_shadowed_manual_tags && ' This service pruned their manual copies.'}
                            </span>
                        </span>
                    </label>
                    <AlertDialogFooter>
                        <AlertDialogCancel>Cancel</AlertDialogCancel>
                        <Button variant={dialog === 'delete' ? 'destructive' : 'default'} onClick={confirm} disabled={pending}>
                            {dialog === 'disable' ? 'Disable' : 'Delete'}
                        </Button>
                    </AlertDialogFooter>
                </AlertDialogContent>
            </AlertDialog>
        </span>
    )
}
