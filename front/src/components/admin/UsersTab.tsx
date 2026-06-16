import {useState} from 'react'
import {Loader2, MoreHorizontal, Plus, RefreshCw, Shield, ShieldOff, Trash2, User} from 'lucide-react'
import {toast} from 'sonner'
import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {z} from 'zod'
import {Table, TableBody, TableCell, TableHead, TableHeader, TableRow} from '@/components/ui/table'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger} from '@/components/ui/dialog'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger} from '@/components/ui/dropdown-menu'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Skeleton} from '@/components/ui/skeleton'
import {Checkbox} from '@/components/ui/checkbox'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useAdminUserMutations, useAdminUsers, useUserShares, useUserStats} from '@/hooks/useAdmin'
import {apiErrorMessage} from '@/api/client'
import type {AdminUserResponse} from '@/lib/types'

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    const i = Math.floor(Math.log(bytes) / Math.log(1024))
    return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`
}

// ---------- Create user dialog ----------

const createSchema = z.object({
    username: z.string().min(1, 'Required').regex(/^[A-Za-z0-9_]+$/, 'Only letters, digits and _'),
    email: z.string().email('Valid email required'),
    display_name: z.string().min(1, 'Required'),
    password: z.string().min(8, 'At least 8 characters'),
    is_admin: z.boolean(),
})
type CreateForm = z.infer<typeof createSchema>

function CreateUserDialog() {
    const [open, setOpen] = useState(false)
    const {create} = useAdminUserMutations()

    const {register, handleSubmit, reset, setValue, watch, formState: {errors}} = useForm<CreateForm>({
        resolver: zodResolver(createSchema),
        defaultValues: {username: '', email: '', display_name: '', password: '', is_admin: false},
    })

    const isAdmin = watch('is_admin')

    const onSubmit = async (values: CreateForm) => {
        try {
            await create.mutateAsync(values)
            toast.success(`User @${values.username} created`)
            reset()
            setOpen(false)
        } catch (e) {
            toast.error('Failed to create user', {description: apiErrorMessage(e)})
        }
    }

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>
                <Button size="sm" className="gap-2">
                    <Plus className="h-4 w-4"/>
                    New user
                </Button>
            </DialogTrigger>
            <DialogContent>
                <DialogHeader>
                    <DialogTitle>Create user</DialogTitle>
                </DialogHeader>
                <form onSubmit={handleSubmit(onSubmit)} className="space-y-4 pt-2" noValidate>
                    <div className="space-y-1.5">
                        <Label htmlFor="username">Username</Label>
                        <Input id="username" {...register('username')} placeholder="alice"/>
                        {errors.username && <p className="text-xs text-destructive">{errors.username.message}</p>}
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="display_name">Display name</Label>
                        <Input id="display_name" {...register('display_name')} placeholder="Alice"/>
                        {errors.display_name && <p className="text-xs text-destructive">{errors.display_name.message}</p>}
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="email">Email</Label>
                        <Input id="email" type="email" {...register('email')} placeholder="alice@example.com"/>
                        {errors.email && <p className="text-xs text-destructive">{errors.email.message}</p>}
                    </div>
                    <div className="space-y-1.5">
                        <Label htmlFor="password">Password</Label>
                        <Input id="password" type="password" {...register('password')}/>
                        {errors.password && <p className="text-xs text-destructive">{errors.password.message}</p>}
                    </div>
                    <div className="flex items-center gap-2">
                        <Checkbox
                            id="is_admin"
                            checked={isAdmin}
                            onCheckedChange={(v) => setValue('is_admin', !!v)}
                        />
                        <Label htmlFor="is_admin">Grant admin role</Label>
                    </div>
                    <Button type="submit" className="w-full" disabled={create.isPending}>
                        {create.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                        Create
                    </Button>
                </form>
            </DialogContent>
        </Dialog>
    )
}

// ---------- Edit user dialog ----------

const editSchema = z.object({
    display_name: z.string().min(1, 'Required'),
    is_admin: z.boolean(),
})
type EditForm = z.infer<typeof editSchema>

function EditUserDialog({user, children}: { user: AdminUserResponse; children: React.ReactNode }) {
    const [open, setOpen] = useState(false)
    const {update} = useAdminUserMutations()

    const {register, handleSubmit, setValue, watch, formState: {errors}} = useForm<EditForm>({
        resolver: zodResolver(editSchema),
        defaultValues: {display_name: user.display_name, is_admin: user.is_admin},
    })

    const isAdmin = watch('is_admin')

    const onSubmit = async (values: EditForm) => {
        try {
            await update.mutateAsync({id: user.id, body: values})
            toast.success('User updated')
            setOpen(false)
        } catch (e) {
            toast.error('Failed to update user', {description: apiErrorMessage(e)})
        }
    }

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>{children}</DialogTrigger>
            <DialogContent>
                <DialogHeader>
                    <DialogTitle>Edit @{user.username}</DialogTitle>
                </DialogHeader>
                <form onSubmit={handleSubmit(onSubmit)} className="space-y-4 pt-2" noValidate>
                    <div className="space-y-1.5">
                        <Label htmlFor="edit_display_name">Display name</Label>
                        <Input id="edit_display_name" {...register('display_name')}/>
                        {errors.display_name && <p className="text-xs text-destructive">{errors.display_name.message}</p>}
                    </div>
                    <div className="flex items-center gap-2">
                        <Checkbox
                            id="edit_is_admin"
                            checked={isAdmin}
                            onCheckedChange={(v) => setValue('is_admin', !!v)}
                        />
                        <Label htmlFor="edit_is_admin">Admin role</Label>
                    </div>
                    <Button type="submit" className="w-full" disabled={update.isPending}>
                        {update.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                        Save
                    </Button>
                </form>
            </DialogContent>
        </Dialog>
    )
}

// ---------- User detail drawer ----------

function UserDetailDialog({user, children}: { user: AdminUserResponse; children: React.ReactNode }) {
    const [open, setOpen] = useState(false)
    const {data: stats, isLoading: statsLoading} = useUserStats(open ? user.id : null)
    const {data: shares, isLoading: sharesLoading} = useUserShares(open ? user.id : null)

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>{children}</DialogTrigger>
            <DialogContent className="max-w-lg">
                <DialogHeader>
                    <DialogTitle>@{user.username}</DialogTitle>
                </DialogHeader>
                <div className="space-y-4 pt-2 text-sm">
                    <dl className="grid grid-cols-2 gap-2">
                        <dt className="text-muted-foreground">Email</dt>
                        <dd>{user.email}</dd>
                        <dt className="text-muted-foreground">Display name</dt>
                        <dd>{user.display_name}</dd>
                        <dt className="text-muted-foreground">Role</dt>
                        <dd>{user.is_admin ? <Badge variant="secondary">Admin</Badge> : 'User'}</dd>
                        <dt className="text-muted-foreground">Storage</dt>
                        <dd>{formatBytes(user.storage_bytes)}</dd>
                    </dl>

                    {statsLoading ? (
                        <Skeleton className="h-32 w-full"/>
                    ) : stats ? (
                        <>
                            <div className="border-t pt-3">
                                <p className="font-medium mb-2">Stats</p>
                                <dl className="grid grid-cols-2 gap-1.5 text-xs">
                                    <dt className="text-muted-foreground">Owned pictures</dt>
                                    <dd>{stats.owned_picture_count.toLocaleString()}</dd>
                                    <dt className="text-muted-foreground">Received pictures</dt>
                                    <dd>{stats.received_picture_count.toLocaleString()}</dd>
                                    <dt className="text-muted-foreground">Dirty pictures</dt>
                                    <dd>{stats.dirty_picture_count}</dd>
                                    <dt className="text-muted-foreground">Errored shares</dt>
                                    <dd>{stats.errored_share_count}</dd>
                                    <dt className="text-muted-foreground">Jobs pending/running</dt>
                                    <dd>{stats.job_counts.pending}/{stats.job_counts.processing}</dd>
                                </dl>
                            </div>
                        </>
                    ) : null}

                    {!sharesLoading && shares && (
                        <div className="border-t pt-3">
                            <p className="font-medium mb-2">Shares</p>
                            <p className="text-xs text-muted-foreground">
                                {shares.outgoing.length} outgoing · {shares.incoming.length} incoming
                            </p>
                        </div>
                    )}
                </div>
            </DialogContent>
        </Dialog>
    )
}

// ---------- Row actions menu ----------

function UserActionsMenu({user}: { user: AdminUserResponse }) {
    const {remove, wake, update} = useAdminUserMutations()

    const toggleAdmin = async () => {
        try {
            await update.mutateAsync({id: user.id, body: {is_admin: !user.is_admin}})
            toast.success(user.is_admin ? 'Admin role removed' : 'Admin role granted')
        } catch (e) {
            toast.error('Failed to update role', {description: apiErrorMessage(e)})
        }
    }

    const handleWake = async () => {
        try {
            await wake.mutateAsync(user.id)
            toast.success('Pipeline woken')
        } catch (e) {
            toast.error('Failed to wake pipeline', {description: apiErrorMessage(e)})
        }
    }

    const handleDelete = async () => {
        try {
            await remove.mutateAsync(user.id)
            toast.success(`@${user.username} deleted`)
        } catch (e) {
            toast.error('Failed to delete user', {description: apiErrorMessage(e)})
        }
    }

    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon" className="h-8 w-8">
                    <MoreHorizontal className="h-4 w-4"/>
                </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
                <EditUserDialog user={user}>
                    <DropdownMenuItem onSelect={(e) => e.preventDefault()}>
                        <User className="mr-2 h-4 w-4"/>
                        Edit
                    </DropdownMenuItem>
                </EditUserDialog>
                <DropdownMenuItem onClick={toggleAdmin}>
                    {user.is_admin ? (
                        <><ShieldOff className="mr-2 h-4 w-4"/>Remove admin</>
                    ) : (
                        <><Shield className="mr-2 h-4 w-4"/>Grant admin</>
                    )}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={handleWake}>
                    <RefreshCw className="mr-2 h-4 w-4"/>
                    Wake pipeline
                </DropdownMenuItem>
                <DropdownMenuSeparator/>
                <ConfirmDialog
                    trigger={
                        <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onSelect={(e) => e.preventDefault()}
                        >
                            <Trash2 className="mr-2 h-4 w-4"/>
                            Delete user
                        </DropdownMenuItem>
                    }
                    title={`Delete @${user.username}?`}
                    description="This permanently removes the user and all their data."
                    confirmLabel="Delete"
                    destructive
                    onConfirm={handleDelete}
                />
            </DropdownMenuContent>
        </DropdownMenu>
    )
}

// ---------- Tab ----------

export function UsersTab() {
    const {data: users, isLoading} = useAdminUsers()

    return (
        <div className="space-y-4">
            <div className="flex items-center justify-between">
                <p className="text-sm text-muted-foreground">
                    {users ? `${users.length} user${users.length !== 1 ? 's' : ''}` : ''}
                </p>
                <CreateUserDialog/>
            </div>

            <div className="rounded-md border">
                <Table>
                    <TableHeader>
                        <TableRow>
                            <TableHead>Username</TableHead>
                            <TableHead>Display name</TableHead>
                            <TableHead>Email</TableHead>
                            <TableHead>Storage</TableHead>
                            <TableHead>Role</TableHead>
                            <TableHead className="w-10"/>
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {isLoading ? (
                            Array.from({length: 4}).map((_, i) => (
                                <TableRow key={i}>
                                    {Array.from({length: 6}).map((_, j) => (
                                        <TableCell key={j}><Skeleton className="h-4 w-full"/></TableCell>
                                    ))}
                                </TableRow>
                            ))
                        ) : users?.length === 0 ? (
                            <TableRow>
                                <TableCell colSpan={6} className="text-center text-muted-foreground py-8">
                                    No users found
                                </TableCell>
                            </TableRow>
                        ) : (
                            users?.map((user) => (
                                <TableRow key={user.id}>
                                    <TableCell>
                                        <UserDetailDialog user={user}>
                                            <button className="font-mono text-sm hover:underline">
                                                @{user.username}
                                            </button>
                                        </UserDetailDialog>
                                    </TableCell>
                                    <TableCell>{user.display_name}</TableCell>
                                    <TableCell className="text-muted-foreground text-sm">{user.email}</TableCell>
                                    <TableCell className="text-sm">{formatBytes(user.storage_bytes)}</TableCell>
                                    <TableCell>
                                        {user.is_admin && (
                                            <Badge variant="secondary" className="bg-sky-500/15 text-sky-400 border-0">
                                                Admin
                                            </Badge>
                                        )}
                                    </TableCell>
                                    <TableCell>
                                        <UserActionsMenu user={user}/>
                                    </TableCell>
                                </TableRow>
                            ))
                        )}
                    </TableBody>
                </Table>
            </div>
        </div>
    )
}
