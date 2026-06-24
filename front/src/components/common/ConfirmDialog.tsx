import {type ReactNode, useRef} from 'react'
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
    const actionRef = useRef<HTMLButtonElement>(null)

    return (
        <AlertDialog>
            <AlertDialogTrigger asChild>{trigger}</AlertDialogTrigger>
            <AlertDialogContent
                onKeyDown={(e) => {
                    // Radix focuses Cancel by default (safer default), but Enter should always
                    // validate the dialog regardless of which button currently has focus.
                    if (e.key !== 'Enter') return
                    e.preventDefault()
                    actionRef.current?.click()
                }}
            >
                <AlertDialogHeader>
                    <AlertDialogTitle>{title}</AlertDialogTitle>
                    {description && <AlertDialogDescription>{description}</AlertDialogDescription>}
                </AlertDialogHeader>
                <AlertDialogFooter>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                    <AlertDialogAction
                        ref={actionRef}
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
