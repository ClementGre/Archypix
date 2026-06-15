import type {ReactNode} from 'react'
import {
    AlertDialog,
    AlertDialogAction,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
    AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import {cn} from '@/lib/utils'

/** Confirmation gate for sensitive actions (revoke / reject / delete …). */
export function ConfirmDialog({
                                  trigger,
                                  title,
                                  description,
                                  confirmLabel = 'Confirm',
                                  destructive = false,
                                  onConfirm,
                              }: {
    trigger: ReactNode
    title: string
    description?: string
    confirmLabel?: string
    destructive?: boolean
    onConfirm: () => void
}) {
    return (
        <AlertDialog>
            <AlertDialogTrigger asChild>{trigger}</AlertDialogTrigger>
            <AlertDialogContent>
                <AlertDialogHeader>
                    <AlertDialogTitle>{title}</AlertDialogTitle>
                    {description && <AlertDialogDescription>{description}</AlertDialogDescription>}
                </AlertDialogHeader>
                <AlertDialogFooter>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                    <AlertDialogAction
                        onClick={onConfirm}
                        className={cn(destructive && 'bg-destructive text-destructive-foreground hover:bg-destructive/90')}
                    >
                        {confirmLabel}
                    </AlertDialogAction>
                </AlertDialogFooter>
            </AlertDialogContent>
        </AlertDialog>
    )
}
