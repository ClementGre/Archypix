import {type ReactNode, useEffect, useRef, useState} from 'react'
import {AlertTriangle, Loader2} from 'lucide-react'
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
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import type {BatchDryRun} from '@/lib/types'

interface BatchConfirmDialogProps {
    /** Trigger element (uncontrolled mode). Omit when driving `open`/`onOpenChange` externally. */
    trigger?: ReactNode
    /** Controlled open state (e.g. opened programmatically after a tag is picked). */
    open?: boolean
    onOpenChange?: (open: boolean) => void
    title: string
    description?: string
    confirmLabel?: string
    destructive?: boolean
    /** Runs once when the dialog opens (and whenever `dryRunKey` changes) to preview the apply (§6.1). */
    dryRun: () => Promise<BatchDryRun>
    /** Re-runs the dry-run when this changes (e.g. an EXIF mode toggle alters the breakdown). */
    dryRunKey?: string
    /** Renders the resolved breakdown; defaults to "<n> photos affected". */
    renderResult?: (result: BatchDryRun) => ReactNode
    /** Extra controls rendered above the result (e.g. a mode selector). */
    children?: ReactNode
    /** Confirm is disabled when the previewed `affected` is 0 unless this is set. */
    allowEmpty?: boolean
    onConfirm: () => void
}

/**
 * The mandatory confirmation gate for a batch write (§6). On open it runs the endpoint's
 * `dry_run` to preview the exact effect (using the same resolution as the apply, so the count
 * cannot diverge), then enables Confirm.
 */
export function BatchConfirmDialog({
                                       trigger,
                                       open: openProp,
                                       onOpenChange,
                                       title,
                                       description,
                                       confirmLabel = 'Confirm',
                                       destructive = false,
                                       dryRun,
                                       dryRunKey,
                                       renderResult,
                                       children,
                                       allowEmpty = false,
                                       onConfirm,
                                   }: BatchConfirmDialogProps) {
    const [openState, setOpenState] = useState(false)
    const controlled = openProp !== undefined
    const open = controlled ? openProp : openState
    const setOpen = (v: boolean) => (controlled ? onOpenChange?.(v) : setOpenState(v))
    const [result, setResult] = useState<BatchDryRun | null>(null)
    const [loading, setLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)

    // Read the latest dryRun fn without re-triggering the effect on every render.
    const dryRunRef = useRef(dryRun)
    dryRunRef.current = dryRun

    useEffect(() => {
        if (!open) {
            setResult(null)
            setError(null)
            return
        }
        let cancelled = false
        setLoading(true)
        setError(null)
        setResult(null)
        dryRunRef
            .current()
            .then((r) => !cancelled && setResult(r))
            .catch((e) => !cancelled && setError(apiErrorMessage(e)))
            .finally(() => !cancelled && setLoading(false))
        return () => {
            cancelled = true
        }
    }, [open, dryRunKey])

    const confirm = () => {
        setOpen(false)
        onConfirm()
    }

    const disabled = loading || !!error || (!allowEmpty && (result?.affected ?? 0) === 0)

    return (
        <AlertDialog open={open} onOpenChange={setOpen}>
            {trigger && <AlertDialogTrigger asChild>{trigger}</AlertDialogTrigger>}
            <AlertDialogContent>
                <AlertDialogHeader>
                    <AlertDialogTitle>{title}</AlertDialogTitle>
                    {description && <AlertDialogDescription>{description}</AlertDialogDescription>}
                </AlertDialogHeader>

                {children}

                <div className="min-h-[1.5rem] text-sm">
                    {loading && (
                        <span className="flex items-center gap-2 text-muted-foreground">
                            <Loader2 className="h-4 w-4 animate-spin"/> Calculating…
                        </span>
                    )}
                    {error && (
                        <span className="flex items-center gap-2 text-destructive">
                            <AlertTriangle className="h-4 w-4"/> {error}
                        </span>
                    )}
                    {!loading && !error && result && (
                        renderResult ? (
                            renderResult(result)
                        ) : (
                            <span>
                                <span className="font-medium tabular-nums">{result.affected}</span>{' '}
                                {result.affected === 1 ? 'photo' : 'photos'} affected.
                            </span>
                        )
                    )}
                </div>

                <AlertDialogFooter>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                    <AlertDialogAction
                        onClick={confirm}
                        disabled={disabled}
                        className={cn(destructive && 'bg-destructive text-destructive-foreground hover:bg-destructive/90')}
                    >
                        {confirmLabel}
                    </AlertDialogAction>
                </AlertDialogFooter>
            </AlertDialogContent>
        </AlertDialog>
    )
}
