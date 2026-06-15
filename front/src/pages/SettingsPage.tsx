import {useEffect} from 'react'
import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {z} from 'zod'
import {Loader2} from 'lucide-react'
import {toast} from 'sonner'
import {Card, CardContent, CardHeader, CardTitle} from '@/components/ui/card'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Button} from '@/components/ui/button'
import {RadioGroup, RadioGroupItem} from '@/components/ui/radio-group'
import {Skeleton} from '@/components/ui/skeleton'
import {useAuthStore} from '@/stores/auth'
import {apiErrorMessage} from '@/api/client'
import {useSettings, useUpdateProfile, useUpdateSettings} from '@/hooks/useSettings'
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

    return (
        <Card>
            <CardHeader>
                <CardTitle>Library</CardTitle>
            </CardHeader>
            <CardContent>
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
                <LibraryCard/>
            </div>
        </div>
    )
}
