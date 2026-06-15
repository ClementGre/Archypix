import {useState} from 'react'
import {Trash2} from 'lucide-react'
import {
    AlertDialog,
    AlertDialogCancel,
    AlertDialogContent,
    AlertDialogDescription,
    AlertDialogFooter,
    AlertDialogHeader,
    AlertDialogTitle,
    AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import {Button} from '@/components/ui/button'

interface DeleteServiceDialogProps {
    onDelete: (promoteTags: boolean) => void
    isPending: boolean
}

export function DeleteServiceDialog({onDelete, isPending}: DeleteServiceDialogProps) {
    const [open, setOpen] = useState(false)

    const handle = (promoteTags: boolean) => {
        onDelete(promoteTags)
        setOpen(false)
    }

    return (
        <AlertDialog open={open} onOpenChange={setOpen}>
            <AlertDialogTrigger asChild>
                <Button variant="ghost" size="icon" className="h-7 w-7 text-muted-foreground hover:text-destructive">
                    <Trash2 className="h-3.5 w-3.5"/>
                </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
                <AlertDialogHeader>
                    <AlertDialogTitle>Delete service</AlertDialogTitle>
                    <AlertDialogDescription>
                        What should happen to the tags this service assigned?
                    </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter className="flex-col gap-2 sm:flex-col">
                    <Button
                        variant="default"
                        onClick={() => handle(true)}
                        disabled={isPending}
                    >
                        Promote tags to manual
                    </Button>
                    <Button
                        variant="destructive"
                        onClick={() => handle(false)}
                        disabled={isPending}
                    >
                        Remove assigned tags
                    </Button>
                    <AlertDialogCancel>Cancel</AlertDialogCancel>
                </AlertDialogFooter>
            </AlertDialogContent>
        </AlertDialog>
    )
}
