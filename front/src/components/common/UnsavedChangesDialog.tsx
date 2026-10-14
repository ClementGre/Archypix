import {useState} from 'react'
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
import {useUnsavedChanges} from '@/stores/unsavedChanges'

/** Asks what to do with an editor's unsaved changes before a guarded navigation runs. */
export function UnsavedChangesDialog() {
    const pending = useUnsavedChanges((s) => s.pending)
    const settle = useUnsavedChanges((s) => s.settle)
    const [saving, setSaving] = useState(false)

    const save = async () => {
        setSaving(true)
        await settle('save')
        setSaving(false)
    }

    return (
        <AlertDialog open={pending !== null} onOpenChange={(o) => !o && !saving && void settle('cancel')}>
            <AlertDialogContent>
                <AlertDialogHeader>
                    <AlertDialogTitle>Unsaved changes</AlertDialogTitle>
                    <AlertDialogDescription>Save them before leaving?</AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                    <AlertDialogCancel disabled={saving}>Cancel</AlertDialogCancel>
                    <Button variant="outline" onClick={() => void settle('discard')} disabled={saving}>
                        Discard
                    </Button>
                    <Button onClick={() => void save()} disabled={saving}>
                        Save
                    </Button>
                </AlertDialogFooter>
            </AlertDialogContent>
        </AlertDialog>
    )
}
