import {useState} from 'react'
import {HardDrive, Info, Loader2, MoreHorizontal, Plus, RefreshCw, Shield, ShieldOff, Trash2, User,} from 'lucide-react'
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
import {NumberInput} from '@/components/ui/number-input'
import {Label} from '@/components/ui/label'
import {Skeleton} from '@/components/ui/skeleton'
import {Checkbox} from '@/components/ui/checkbox'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {STORAGE_SEGMENT_CLASS, StorageBar} from '@/components/StorageBar'
import {useAdminUserMutations, useAdminUsers, useUserShares, useUserStats, useUserStorageAudit} from '@/hooks/useAdmin'
import {apiErrorMessage} from '@/api/client'
import {cn, formatBytes} from '@/lib/utils'
import type {AdminUserResponse} from '@/lib/types'

const BYTES_PER_GB = 1024 ** 3

/** `formatBytes` reads a real zero as "—"; admin counters are always known, so show "0 B" instead. */
function formatUsage(bytes: number): string {
    return bytes === 0 ? '0 B' : formatBytes(bytes)
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
    quota_unlimited: z.boolean(),
    quota_gb: z.number().min(0, 'Must be ≥ 0'),
})
type EditForm = z.infer<typeof editSchema>

function EditUserDialog({user, children}: { user: AdminUserResponse; children: React.ReactNode }) {
    const [open, setOpen] = useState(false)
    const {update} = useAdminUserMutations()

    const {register, handleSubmit, setValue, watch, formState: {errors}} = useForm<EditForm>({
        resolver: zodResolver(editSchema),
        defaultValues: {
            display_name: user.display_name,
            is_admin: user.is_admin,
            quota_unlimited: user.quota_bytes == null,
            quota_gb: user.quota_bytes ? user.quota_bytes / BYTES_PER_GB : 0,
        },
    })

    const isAdmin = watch('is_admin')
    const quotaUnlimited = watch('quota_unlimited')

    const onSubmit = async (values: EditForm) => {
        try {
            await update.mutateAsync({
                id: user.id,
                body: {
                    display_name: values.display_name,
                    is_admin: values.is_admin,
                    storage_quota_bytes: values.quota_unlimited
                        ? null
                        : Math.round(values.quota_gb * BYTES_PER_GB),
                },
            })
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
                    <div className="space-y-1.5 border-t pt-3">
                        <Label>Storage quota</Label>
                        <div className="flex items-center gap-2">
                            <Checkbox
                                id="edit_quota_unlimited"
                                checked={quotaUnlimited}
                                onCheckedChange={(v) => setValue('quota_unlimited', !!v)}
                            />
                            <Label htmlFor="edit_quota_unlimited" className="font-normal">Unlimited</Label>
                        </div>
                        {!quotaUnlimited && (
                            <div className="flex items-center gap-2">
                                <NumberInput
                                    step="0.1"
                                    min={0}
                                    {...register('quota_gb', {valueAsNumber: true})}
                                    className="max-w-[10rem]"
                                />
                                <span className="text-sm text-muted-foreground">GB</span>
                            </div>
                        )}
                        {errors.quota_gb && <p className="text-xs text-destructive">{errors.quota_gb.message}</p>}
                        <p className="text-xs text-muted-foreground">
                            Currently using {formatUsage(user.storage_bytes)}. Lowering the quota below current usage
                            blocks new writes but never deletes stored data.
                        </p>
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

// ---------- Storage audit dialog ----------

function BreakdownLine({label, bytes, swatchClassName}: { label: string; bytes: number; swatchClassName: string }) {
    return (
        <div className="flex items-center justify-between text-xs">
            <span className="flex items-center gap-1.5 text-muted-foreground">
                <span className={cn('h-2 w-2 shrink-0 rounded-sm', swatchClassName)}/>
                {label}
            </span>
            <span className="tabular-nums">{formatUsage(bytes)}</span>
        </div>
    )
}

function StorageAuditDialog({user, children}: { user: AdminUserResponse; children: React.ReactNode }) {
    const [open, setOpen] = useState(false)
    const {data: audit, isLoading} = useUserStorageAudit(open ? user.id : null)

    const s3TotalBytes = audit
        ? audit.buckets.pictures.total_bytes
        + audit.buckets.versions.total_bytes
        + audit.thumbnails_bytes
        + audit.buckets.staging.total_bytes
        : 0
    const thumbnailsVsPicturesPct = audit && audit.buckets.pictures.total_bytes > 0
        ? (audit.thumbnails_bytes / audit.buckets.pictures.total_bytes) * 100
        : null

    return (
        <Dialog open={open} onOpenChange={setOpen}>
            <DialogTrigger asChild>{children}</DialogTrigger>
            <DialogContent className="max-w-lg">
                <DialogHeader>
                    <DialogTitle>Storage audit — @{user.username}</DialogTitle>
                </DialogHeader>
                {isLoading || !audit ? (
                    <Skeleton className="h-64 w-full"/>
                ) : (
                    <div className="space-y-4 pt-2 text-sm">
                        <div className="grid grid-cols-2 gap-1.5 text-xs">
                            <span className="text-muted-foreground">DB billed</span>
                            <span className="text-right tabular-nums">{formatUsage(audit.db_billed_bytes)}</span>
                            <span className="text-muted-foreground">S3 measured (originals + versions)</span>
                            <span className="text-right tabular-nums">{formatUsage(audit.s3_billed_bytes)}</span>
                            <span className="text-muted-foreground">Drift</span>
                            <span
                                className={cn(
                                    'text-right tabular-nums',
                                    audit.drift_bytes !== 0 && 'text-amber-600 dark:text-amber-500',
                                )}
                            >
                                {audit.drift_bytes > 0 ? '+' : ''}
                                {formatUsage(audit.drift_bytes)}
                            </span>
                            <span className="text-muted-foreground">Thumbnails total (free, untracked)</span>
                            <span className="text-right tabular-nums">
                                {formatUsage(audit.thumbnails_bytes)}
                                {thumbnailsVsPicturesPct != null && (
                                    <span className="text-muted-foreground"> ({thumbnailsVsPicturesPct.toFixed(1)}% of pictures)</span>
                                )}
                            </span>
                            <span className="text-muted-foreground">S3 total (all buckets)</span>
                            <span className="text-right tabular-nums">{formatUsage(s3TotalBytes)}</span>
                        </div>

                        <div className="space-y-1.5 border-t pt-3">
                            <p className="font-medium text-xs text-muted-foreground">DB breakdown</p>
                            <BreakdownLine label="Originals" bytes={audit.db_breakdown.originals_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.originals}/>
                            <BreakdownLine label="Versions" bytes={audit.db_breakdown.versions_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.versions}/>
                            <BreakdownLine label="Trashed originals" bytes={audit.db_breakdown.originals_trashed_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                            <BreakdownLine label="Trashed versions" bytes={audit.db_breakdown.versions_trashed_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                        </div>

                        <div className="space-y-1.5 border-t pt-3">
                            <p className="font-medium text-xs text-muted-foreground">S3 buckets</p>
                            <Table>
                                <TableHeader>
                                    <TableRow>
                                        <TableHead className="h-7">Bucket</TableHead>
                                        <TableHead className="h-7 text-right">Objects</TableHead>
                                        <TableHead className="h-7 text-right">Bytes</TableHead>
                                    </TableRow>
                                </TableHeader>
                                <TableBody>
                                    {(
                                        [
                                            ['Pictures', audit.buckets.pictures],
                                            ['Versions', audit.buckets.versions],
                                            ['Thumbnails (small)', audit.buckets.thumbnails_small],
                                            ['Thumbnails (medium)', audit.buckets.thumbnails_medium],
                                            ['Thumbnails (large)', audit.buckets.thumbnails_large],
                                            ['Staging', audit.buckets.staging],
                                        ] as const
                                    ).map(([label, usage]) => (
                                        <TableRow key={label}>
                                            <TableCell className="py-1.5 text-xs">{label}</TableCell>
                                            <TableCell className="py-1.5 text-right text-xs tabular-nums">
                                                {usage.object_count.toLocaleString()}
                                            </TableCell>
                                            <TableCell className="py-1.5 text-right text-xs tabular-nums">
                                                {formatUsage(usage.total_bytes)}
                                            </TableCell>
                                        </TableRow>
                                    ))}
                                </TableBody>
                            </Table>
                        </div>
                    </div>
                )}
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
                    </dl>

                    <div className="space-y-1.5 border-t pt-3">
                        <div className="flex items-center justify-between text-xs">
                            <span>
                                <span className="font-medium tabular-nums">{formatUsage(user.storage_bytes)}</span>
                                {user.quota_bytes ? (
                                    <span className="text-muted-foreground"> of {formatBytes(user.quota_bytes)}</span>
                                ) : (
                                    <span className="text-muted-foreground"> · unlimited</span>
                                )}
                            </span>
                            {user.usage_ratio != null && (
                                <span className="tabular-nums text-muted-foreground">
                                    {Math.round(user.usage_ratio * 100)}%
                                </span>
                            )}
                        </div>
                        <StorageBar breakdown={user.breakdown} quotaBytes={user.quota_bytes} usedBytes={user.storage_bytes}/>
                        <div className="grid grid-cols-2 gap-x-6 gap-y-1 pt-1">
                            <BreakdownLine label="Originals" bytes={user.breakdown.originals_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.originals}/>
                            <BreakdownLine label="Versions" bytes={user.breakdown.versions_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.versions}/>
                            <BreakdownLine label="Trashed originals" bytes={user.breakdown.originals_trashed_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                            <BreakdownLine label="Trashed versions" bytes={user.breakdown.versions_trashed_bytes}
                                           swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                        </div>
                        <div className="flex gap-2 pt-1">
                            <EditUserDialog user={user}>
                                <Button variant="outline" size="sm" className="h-7 text-xs">Edit quota</Button>
                            </EditUserDialog>
                            <StorageAuditDialog user={user}>
                                <Button variant="outline" size="sm" className="h-7 gap-1 text-xs">
                                    <HardDrive className="h-3 w-3"/>
                                    Storage audit
                                </Button>
                            </StorageAuditDialog>
                        </div>
                    </div>

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
                <StorageAuditDialog user={user}>
                    <DropdownMenuItem onSelect={(e) => e.preventDefault()}>
                        <HardDrive className="mr-2 h-4 w-4"/>
                        Storage audit
                    </DropdownMenuItem>
                </StorageAuditDialog>
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

/**
 * Single-instance user management. Transport comes from `useAdminClient` (the enclosing
 * `AdminClientProvider`), so the fleet dashboard reuses this verbatim per backend by wrapping it in a
 * proxy provider (feature 24) — `title` labels which backend, `showCreate` can hide the create button.
 */
export function UsersTab({title, showCreate = true}: { title?: React.ReactNode; showCreate?: boolean } = {}) {
    const {data: users, isLoading} = useAdminUsers()

    return (
        <div className="space-y-4">
            <div className="flex items-center justify-between gap-3">
                <p className="text-sm text-muted-foreground">
                    {title ? <span className="font-mono font-medium text-foreground">{title}</span> : null}
                    {title ? ' · ' : ''}
                    {users ? `${users.length} user${users.length !== 1 ? 's' : ''}` : ''}
                </p>
                {showCreate && <CreateUserDialog/>}
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
                                    <TableCell className="min-w-[10rem]">
                                        <div className="flex items-center gap-2">
                                            <div className="w-24 shrink-0 space-y-1">
                                                <div className="text-xs tabular-nums">
                                                    {formatUsage(user.storage_bytes)}
                                                    {user.quota_bytes ? (
                                                        <span className="text-muted-foreground"> / {formatBytes(user.quota_bytes)}</span>
                                                    ) : (
                                                        <span className="text-muted-foreground"> / ∞</span>
                                                    )}
                                                </div>
                                                <StorageBar
                                                    breakdown={user.breakdown}
                                                    quotaBytes={user.quota_bytes}
                                                    usedBytes={user.storage_bytes}
                                                />
                                            </div>
                                            <StorageAuditDialog user={user}>
                                                <Button variant="ghost" size="icon" className="h-6 w-6 shrink-0" title="Storage audit">
                                                    <Info className="h-3.5 w-3.5"/>
                                                </Button>
                                            </StorageAuditDialog>
                                        </div>
                                    </TableCell>
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
