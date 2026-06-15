import {useState} from 'react'
import {Loader2, Plus} from 'lucide-react'
import {toast} from 'sonner'
import {useMutation, useQueryClient} from '@tanstack/react-query'
import {Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger} from '@/components/ui/dialog'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {TagPicker} from '@/components/tags/TagPicker'
import {createOutgoingShare} from '@/api/shares'
import {apiErrorMessage} from '@/api/client'
import {TagPath} from '@/lib/utils'
import {GLOBAL_DOMAIN} from '@/lib/constants'

const DEFAULT_STATE = {
    tag: '',
    recipientUsername: '',
    recipientInstance: GLOBAL_DOMAIN,
    allowShareBack: true,
    future: true,
}

export function CreateShareDialog() {
    const [open, setOpen] = useState(false)
    const [tag, setTag] = useState(DEFAULT_STATE.tag)
    const [recipientUsername, setRecipientUsername] = useState(DEFAULT_STATE.recipientUsername)
    const [recipientInstance, setRecipientInstance] = useState(DEFAULT_STATE.recipientInstance)
    const [allowShareBack, setAllowShareBack] = useState(DEFAULT_STATE.allowShareBack)
    const [future, setFuture] = useState(DEFAULT_STATE.future)

    const queryClient = useQueryClient()

    const mutation = useMutation({
        mutationFn: createOutgoingShare,
        onSuccess: () => {
            toast.success('Share created')
            void queryClient.invalidateQueries({queryKey: ['shares']})
            setTag(DEFAULT_STATE.tag)
            setRecipientUsername(DEFAULT_STATE.recipientUsername)
            setRecipientInstance(DEFAULT_STATE.recipientInstance)
            setAllowShareBack(DEFAULT_STATE.allowShareBack)
            setFuture(DEFAULT_STATE.future)
            setOpen(false)
        },
        onError: (error) => {
            toast.error('Could not create share', {description: apiErrorMessage(error)})
        },
    })

    const isDisabled =
        !tag || !recipientUsername.trim() || !recipientInstance.trim() || mutation.isPending

    const handleSubmit = (e: React.FormEvent) => {
        e.preventDefault()
        mutation.mutate({
            tag_path: tag,
            recipient_username: recipientUsername.trim(),
            recipient_instance: recipientInstance.trim(),
            allow_share_back: allowShareBack,
            future,
        })
    }

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>
                <Button size="sm" className="gap-1.5">
                    <Plus className="h-3.5 w-3.5"/>
                    New share
                </Button>
            </DialogTrigger>
            <DialogContent className="max-w-md">
                <DialogHeader>
                    <DialogTitle>Create outgoing share</DialogTitle>
                </DialogHeader>
                <form onSubmit={handleSubmit} className="space-y-4">
                    {/* Tag picker */}
                    <div className="space-y-1.5">
                        <Label>Tag</Label>
                        <div className="flex items-center gap-2">
                            <TagPicker
                                onSelect={setTag}
                                triggerLabel={tag ? 'Change tag' : 'Choose tag'}
                                allowCreate={false}
                                allowProtected
                            />
                            {tag && (
                                <span className="truncate text-sm text-muted-foreground">{TagPath.toDisplay(tag)}</span>
                            )}
                        </div>
                    </div>

                    {/* Recipient */}
                    <div className="space-y-1.5">
                        <Label htmlFor="recipient-username">Recipient username</Label>
                        <Input
                            id="recipient-username"
                            value={recipientUsername}
                            onChange={(e) => setRecipientUsername(e.target.value)}
                            placeholder="username"
                            autoCapitalize="none"
                            autoCorrect="off"
                            spellCheck={false}
                        />
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="recipient-instance">Recipient instance</Label>
                        <Input
                            id="recipient-instance"
                            value={recipientInstance}
                            onChange={(e) => setRecipientInstance(e.target.value)}
                            placeholder="instance domain"
                            autoCapitalize="none"
                            autoCorrect="off"
                            spellCheck={false}
                        />
                        <p className="text-xs text-muted-foreground">
                            The recipient handle will be <span className="font-mono">@username:instance</span>.
                        </p>
                    </div>

                    {/* Toggles */}
                    <div className="flex items-center justify-between">
                        <Label htmlFor="allow-share-back">Allow ShareBack</Label>
                        <Switch
                            id="allow-share-back"
                            checked={allowShareBack}
                            onCheckedChange={setAllowShareBack}
                        />
                    </div>
                    <div className="flex items-center justify-between">
                        <Label htmlFor="future">Share future additions</Label>
                        <Switch
                            id="future"
                            checked={future}
                            onCheckedChange={setFuture}
                        />
                    </div>

                    <Button type="submit" className="w-full" disabled={isDisabled}>
                        {mutation.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                        Create share
                    </Button>
                </form>
            </DialogContent>
        </Dialog>
    )
}
