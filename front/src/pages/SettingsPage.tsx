import {useEffect} from 'react'
import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {z} from 'zod'
import {useNavigate} from 'react-router-dom'
import {HardDrive, Loader2, Trash2} from 'lucide-react'
import {toast} from 'sonner'
import {Card, CardContent, CardHeader, CardTitle} from '@/components/ui/card'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Button} from '@/components/ui/button'
import {RadioGroup, RadioGroupItem} from '@/components/ui/radio-group'
import {Skeleton} from '@/components/ui/skeleton'
import {NumberInput} from '@/components/ui/number-input'
import {STORAGE_SEGMENT_CLASS, StorageBar} from '@/components/StorageBar'
import {useAuthStore} from '@/stores/auth'
import {apiErrorMessage} from '@/api/client'
import {useSettings, useStorage, useUpdateProfile, useUpdateSettings} from '@/hooks/useSettings'
import {cn, formatBytes} from '@/lib/utils'
import type {VersioningMode} from '@/lib/types'

// ---------- Profile form ----------

const profileSchema = z.object({
    display_name: z.string().min(1, 'Display name is required'),
    email: z.string().email('Must be a valid email'),
})

type ProfileForm = z.infer<typeof profileSchema>

function ProfileCard() {
    const user = useAuthStore((s) => s.user)
    const instance = useAuthStore((s) => s.instance)
    const updateProfile = useUpdateProfile()

    const {
        register,
        handleSubmit,
        reset,
        formState: {errors},
    } = useForm<ProfileForm>({
        resolver: zodResolver(profileSchema),
        defaultValues: {
            display_name: user?.display_name ?? '',
            email: user?.email ?? '',
        },
    })

    // Sync form when the store user changes (e.g. after successful update)
    useEffect(() => {
        if (user) {
            reset({display_name: user.display_name, email: user.email})
        }
    }, [user, reset])

    const onSubmit = async (values: ProfileForm) => {
        try {
            await updateProfile.mutateAsync(values)
            toast.success('Profile updated')
        } catch (e) {
            toast.error('Could not update profile', {description: apiErrorMessage(e)})
        }
    }

    return (
        <Card>
            <CardHeader>
                <CardTitle>Profile</CardTitle>
            </CardHeader>
            <CardContent>
                {/* Read-only handle */}
                <div className="mb-4 text-sm text-muted-foreground">
                    <span className="font-mono">
                        @{user?.username ?? '…'}:{instance ?? '…'}
                    </span>
                </div>

                <form onSubmit={handleSubmit(onSubmit)} className="space-y-4" noValidate>
                    <div className="space-y-1.5">
                        <Label htmlFor="display_name">Display name</Label>
                        <Input id="display_name" {...register('display_name')} />
                        {errors.display_name && (
                            <p className="text-xs text-destructive">{errors.display_name.message}</p>
                        )}
                    </div>

                    <div className="space-y-1.5">
                        <Label htmlFor="email">Email</Label>
                        <Input id="email" type="email" {...register('email')} />
                        {errors.email && (
                            <p className="text-xs text-destructive">{errors.email.message}</p>
                        )}
                    </div>

                    <Button type="submit" disabled={updateProfile.isPending}>
                        {updateProfile.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                        Save profile
                    </Button>
                </form>
            </CardContent>
        </Card>
    )
}

// ---------- Library / versioning card ----------

const VERSIONING_OPTIONS: { value: VersioningMode; label: string; description: string }[] = [
    {
        value: 'none',
        label: 'No versioning',
        description: 'Never keep previous versions.',
    },
    {
        value: 'original_copy',
        label: 'Original copy',
        description: 'Snapshot the original once, on first edit.',
    },
    {
        value: 'full_versioning',
        label: 'Full versioning',
        description: 'Snapshot before every visual edit.',
    },
]

function LibraryCard() {
    const {data: settings, isLoading} = useSettings()
    const updateSettings = useUpdateSettings()

    const handleVersioningChange = async (value: string) => {
        try {
            await updateSettings.mutateAsync({versioning_mode: value as VersioningMode})
            toast.success('Settings saved')
        } catch (e) {
            toast.error('Could not save settings', {description: apiErrorMessage(e)})
        }
    }

    // Uncontrolled input (keyed by the persisted value, so it re-seeds on refetch) committed on blur,
    // avoiding a setState-in-effect just to mirror server state.
    const commitRetention = async (input: HTMLInputElement) => {
        if (!settings) return
        const n = Math.round(Number(input.value))
        if (!Number.isFinite(n) || n === settings.trash_retention_days) {
            input.value = String(settings.trash_retention_days)
            return
        }
        if (n < 1 || n > 3650) {
            toast.error('Retention must be between 1 and 3650 days')
            input.value = String(settings.trash_retention_days)
            return
        }
        try {
            await updateSettings.mutateAsync({trash_retention_days: n})
            toast.success('Settings saved')
        } catch (e) {
            toast.error('Could not save settings', {description: apiErrorMessage(e)})
            input.value = String(settings.trash_retention_days)
        }
    }

    return (
        <Card>
            <CardHeader>
                <CardTitle>Library</CardTitle>
            </CardHeader>
            <CardContent className="space-y-6">
                <div className="space-y-3">
                    <Label>Versioning mode</Label>
                    {isLoading ? (
                        <div className="space-y-2">
                            <Skeleton className="h-5 w-48"/>
                            <Skeleton className="h-5 w-64"/>
                            <Skeleton className="h-5 w-56"/>
                        </div>
                    ) : (
                        <RadioGroup
                            value={settings?.versioning_mode}
                            onValueChange={handleVersioningChange}
                            disabled={updateSettings.isPending}
                        >
                            {VERSIONING_OPTIONS.map((opt) => (
                                <div key={opt.value} className="flex items-start gap-3">
                                    <RadioGroupItem value={opt.value} id={`versioning-${opt.value}`} className="mt-0.5"/>
                                    <div>
                                        <Label htmlFor={`versioning-${opt.value}`} className="font-medium">
                                            {opt.label}
                                        </Label>
                                        <p className="text-xs text-muted-foreground">{opt.description}</p>
                                    </div>
                                </div>
                            ))}
                        </RadioGroup>
                    )}
                </div>

                <div className="space-y-1.5">
                    <Label htmlFor="trash-retention">Trash retention</Label>
                    {isLoading ? (
                        <Skeleton className="h-10 w-40"/>
                    ) : (
                        <div className="flex items-center gap-2">
                            <NumberInput
                                id="trash-retention"
                                key={settings?.trash_retention_days}
                                className="h-10 w-28"
                                min={1}
                                max={3650}
                                step={1}
                                defaultValue={settings?.trash_retention_days}
                                disabled={updateSettings.isPending}
                                onBlur={(e) => commitRetention(e.target)}
                                onKeyDown={(e) => {
                                    if (e.key === 'Enter') (e.target as HTMLInputElement).blur()
                                }}
                            />
                            <span className="text-sm text-muted-foreground">days</span>
                        </div>
                    )}
                    <p className="text-xs text-muted-foreground">
                        How long your trashed photos are kept before being permanently deleted (1–3650).
                    </p>
                </div>
            </CardContent>
        </Card>
    )
}

// ---------- Storage card (feature 22) ----------

// These byte counts are always known numbers (never "no data yet"), so 0 should read as
// "0 MB" rather than formatBytes' usual "—" for falsy/absent values.
function formatUsage(bytes: number): string {
    return bytes === 0 ? '0 KB' : formatBytes(bytes)
}

function BreakdownRow({label, bytes, swatchClassName}: { label: string; bytes: number; swatchClassName: string }) {
    return (
        <div className="flex items-center justify-between text-sm">
            <span className="flex items-center gap-2 text-muted-foreground">
                <span className={cn('h-2.5 w-2.5 shrink-0 rounded-sm', swatchClassName)}/>
                {label}
            </span>
            <span className="tabular-nums">{formatUsage(bytes)}</span>
        </div>
    )
}

function StorageCard() {
    const navigate = useNavigate()
    const {data: storage, isLoading} = useStorage()

    if (isLoading || !storage) {
        return (
            <Card>
                <CardHeader>
                    <CardTitle>Storage</CardTitle>
                </CardHeader>
                <CardContent>
                    <Skeleton className="h-24 w-full"/>
                </CardContent>
            </Card>
        )
    }

    const {quota_bytes, used_bytes, breakdown, reclaimable_trash_bytes, usage_ratio, warn_level} = storage
    const pct =
        quota_bytes && quota_bytes > 0
            ? Math.min(100, Math.round((usage_ratio ?? 0) * 100))
            : 0

    return (
        <Card>
            <CardHeader>
                <CardTitle className="flex items-center gap-2">
                    <HardDrive className="h-4 w-4"/>
                    Storage
                </CardTitle>
            </CardHeader>
            <CardContent className="space-y-5">
                {/* Usage headline + bar */}
                <div className="space-y-1.5">
                    <div className="flex items-center justify-between text-sm">
                        <span>
                            <span className="font-medium tabular-nums">{formatUsage(used_bytes)}</span>
                            {quota_bytes && quota_bytes > 0 ? (
                                <span className="text-muted-foreground"> of {formatBytes(quota_bytes)} used</span>
                            ) : (
                                <span className="text-muted-foreground"> used · unlimited</span>
                            )}
                        </span>
                        {quota_bytes && quota_bytes > 0 && (
                            <span className="tabular-nums text-muted-foreground">{pct}%</span>
                        )}
                    </div>
                    <StorageBar
                        breakdown={breakdown}
                        quotaBytes={quota_bytes}
                        usedBytes={used_bytes}
                        className="rounded-full"
                    />
                    {warn_level === 'full' && (
                        <p className="text-xs text-destructive">
                            Storage is full. free up space (or empty your trash) before uploading more.
                        </p>
                    )}
                    {warn_level === 'critical' && (
                        <p className="text-xs text-amber-600 dark:text-amber-500">
                            You are almost out of space.
                        </p>
                    )}
                </div>

                {/* Four-cell breakdown */}
                <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2 sm:gap-x-8">
                    <BreakdownRow label="Originals" bytes={breakdown.originals_bytes} swatchClassName={STORAGE_SEGMENT_CLASS.originals}/>
                    <BreakdownRow label="Versions" bytes={breakdown.versions_bytes} swatchClassName={STORAGE_SEGMENT_CLASS.versions}/>
                    <BreakdownRow label="Trashed originals" bytes={breakdown.originals_trashed_bytes}
                                  swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                    <BreakdownRow label="Trashed versions" bytes={breakdown.versions_trashed_bytes} swatchClassName={STORAGE_SEGMENT_CLASS.trashed}/>
                </div>

                {/* Reclaimable trash prompt */}
                {reclaimable_trash_bytes > 0 && (
                    <div className="flex items-center justify-between rounded-md border border-border bg-muted/30 px-3 py-2">
                        <span className="text-sm text-muted-foreground">
                            Empty your trash to reclaim{' '}
                            <span className="font-medium text-foreground">{formatBytes(reclaimable_trash_bytes)}</span>
                        </span>
                        <Button variant="outline" size="sm" className="h-7 gap-1.5" onClick={() => navigate('/trash')}>
                            <Trash2 className="h-3.5 w-3.5"/>
                            Open trash
                        </Button>
                    </div>
                )}
            </CardContent>
        </Card>
    )
}

// ---------- Page ----------

export default function SettingsPage() {
    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-2xl space-y-6">
                <h1 className="text-xl font-semibold">Settings</h1>
                <ProfileCard/>
                <StorageCard/>
                <LibraryCard/>
            </div>
        </div>
    )
}
