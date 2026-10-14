import {type ReactNode, useState} from 'react'
import {toast} from 'sonner'
import {Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle, DialogTrigger,} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {RadioGroup, RadioGroupItem} from '@/components/ui/radio-group'
import {useHierarchyMutations} from '@/hooks/useHierarchies'
import type {HierarchyConfig} from '@/lib/types'
import {cameraSyncConfig, emptyConfig} from './hierarchyUtils'
import {apiErrorMessage} from '@/api/client'

const PRESETS: { value: string; label: string; hint: string; config: () => HierarchyConfig }[] = [
    {value: 'empty', label: 'Empty', hint: 'Add directories yourself.', config: emptyConfig},
    {
        value: 'camera',
        label: 'Phone camera sync',
        hint: 'A Camera folder for a sync app: it lists what it holds so uploads verify, and deleting there only untags.',
        config: cameraSyncConfig,
    },
]

/** Creates a hierarchy from a preset then hands the new id back (to open the editor). */
export function CreateHierarchyDialog({
                                          trigger,
                                          onCreated,
                                      }: {
    trigger: ReactNode
    onCreated: (id: string) => void
}) {
    const [open, setOpen] = useState(false)
    const [name, setName] = useState('')
    const [preset, setPreset] = useState('empty')
    const {create} = useHierarchyMutations()

    const submit = () => {
        const trimmed = name.trim()
        if (!trimmed) return
        create.mutate(
            {name: trimmed, config: (PRESETS.find((p) => p.value === preset) ?? PRESETS[0]).config()},
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
                <RadioGroup value={preset} onValueChange={setPreset} className="space-y-1">
                    {PRESETS.map((p) => (
                        <label key={p.value} className="flex cursor-pointer items-start gap-2 text-sm">
                            <RadioGroupItem value={p.value} className="mt-0.5"/>
                            <span>
                                {p.label}
                                <span className="block text-xs text-muted-foreground">{p.hint}</span>
                            </span>
                        </label>
                    ))}
                </RadioGroup>
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
