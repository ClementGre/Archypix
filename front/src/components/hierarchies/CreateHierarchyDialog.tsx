import {type ReactNode, useState} from 'react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {useHierarchyMutations} from '@/hooks/useHierarchies'
import {emptyConfig} from './hierarchyUtils'
import {apiErrorMessage} from '@/api/client'

/** Creates an empty hierarchy then hands the new id back (to open the editor). */
export function CreateHierarchyDialog({
                                          trigger,
                                          onCreated,
                                      }: {
    trigger: ReactNode
    onCreated: (id: string) => void
}) {
    const [open, setOpen] = useState(false)
    const [name, setName] = useState('')
    const {create} = useHierarchyMutations()

    const submit = () => {
        const trimmed = name.trim()
        if (!trimmed) return
        create.mutate(
            {name: trimmed, config: emptyConfig()},
            {
                onSuccess: (h) => {
                    setOpen(false)
                    setName('')
                    onCreated(h.id)
                },
                onError: (e) => toast.error(apiErrorMessage(e)),
            },
        )
    }

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>{trigger}</DialogTrigger>
            <DialogContent>
                <DialogHeader>
                    <DialogTitle>New hierarchy</DialogTitle>
                    <DialogDescription>
                        A hierarchy is a saved, navigable view of your tag graph. You can add directories after creating it.
                    </DialogDescription>
                </DialogHeader>
                <div className="space-y-2">
                    <Label htmlFor="hierarchy-name">Name</Label>
                    <Input
                        id="hierarchy-name"
                        value={name}
                        autoFocus
                        placeholder="e.g. My library"
                        onChange={(e) => setName(e.target.value)}
                        onKeyDown={(e) => e.key === 'Enter' && submit()}
                    />
                </div>
                <DialogFooter>
                    <Button variant="ghost" onClick={() => setOpen(false)}>
                        Cancel
                    </Button>
                    <Button onClick={submit} disabled={!name.trim() || create.isPending}>
                        Create
                    </Button>
                </DialogFooter>
            </DialogContent>
        </Dialog>
    )
}
