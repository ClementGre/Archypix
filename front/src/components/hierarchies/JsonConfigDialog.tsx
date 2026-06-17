import {type ReactNode, useState} from 'react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Textarea} from '@/components/ui/textarea'
import type {HierarchyConfig} from '@/lib/types'

/** Raw JSON view/edit of the config — a debug escape hatch; validated on the server at save. */
export function JsonConfigDialog({
                                     trigger,
                                     config,
                                     onApply,
                                 }: {
    trigger: ReactNode
    config: HierarchyConfig
    onApply: (config: HierarchyConfig) => void
}) {
    const [open, setOpen] = useState(false)
    const [text, setText] = useState('')

    // Reseed the textarea from the current config each time the dialog opens.
    const onOpenChange = (o: boolean) => {
        if (o) setText(JSON.stringify(config, null, 2))
        setOpen(o)
    }

    const apply = () => {
        try {
            const parsed = JSON.parse(text) as HierarchyConfig
            onApply(parsed)
            setOpen(false)
        } catch (e) {
            toast.error(e instanceof Error ? e.message : 'Invalid JSON')
        }
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogTrigger asChild>{trigger}</DialogTrigger>
            <DialogContent className="max-w-2xl">
                <DialogHeader>
                    <DialogTitle>Config JSON (debug)</DialogTitle>
                    <DialogDescription>
                        Raw <code>config</code> blob. For debugging — the server validates it on save. Applying only
                        updates the draft; remember to Save.
                    </DialogDescription>
                </DialogHeader>
                <Textarea
                    value={text}
                    onChange={(e) => setText(e.target.value)}
                    spellCheck={false}
                    className="h-96 font-mono text-xs"
                />
                <DialogFooter>
                    <Button variant="ghost" onClick={() => setOpen(false)}>
                        Cancel
                    </Button>
                    <Button onClick={apply}>Apply to draft</Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
