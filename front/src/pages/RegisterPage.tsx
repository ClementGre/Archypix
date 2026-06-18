import {useEffect, useState} from 'react'
import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {Link, useNavigate} from 'react-router-dom'
import {Loader2, Pencil} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from '@/components/ui/card'
import {InstanceCorsWarning} from '@/components/common/InstanceCorsWarning'
import {cn} from '@/lib/utils'
import {getPreferredInstance, GLOBAL_DOMAIN, setPreferredInstance} from '@/lib/constants'
import {type RegisterForm, registerFormSchema} from '@/lib/schemas'
import {login, register as registerUser} from '@/api/auth'
import {apiErrorMessage} from '@/api/client'

export default function RegisterPage() {
    const navigate = useNavigate()

    const [editingInstance, setEditingInstance] = useState(false)

    const {
        register,
        handleSubmit,
        watch,
        formState: {errors, isSubmitting},
    } = useForm<RegisterForm>({
        resolver: zodResolver(registerFormSchema),
        defaultValues: {username: '', instance: getPreferredInstance(), display_name: '', email: '', password: ''},
    })

    const instance = watch('instance')

    // Persist the chosen instance so login and register stay in sync.
    useEffect(() => setPreferredInstance(instance), [instance])

    const onSubmit = async (values: RegisterForm) => {
        try {
            const {instance: domain, ...payload} = values
            await registerUser(payload, domain)
            await login(values.username, values.password, domain)
            toast.success('Account created')
            navigate('/', {replace: true})
        } catch (error) {
            toast.error('Registration failed', {description: apiErrorMessage(error)})
        }
    }

    return (
        <div className="flex min-h-screen items-center justify-center bg-background p-6">
            <Card className="w-full max-w-md">
                <CardHeader className="space-y-1">
                    <CardTitle className="text-2xl">Create account</CardTitle>
                    <CardDescription>
                        Registering on <span className="font-medium text-foreground">{instance || GLOBAL_DOMAIN}</span>.
                    </CardDescription>
                </CardHeader>
                <CardContent>
                    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4" noValidate>
                        {/* Handle field: @username:instance, instance editable on click */}
                        <div className="space-y-1.5">
                            <Label>Account</Label>
                            <div
                                className={cn(
                                    'flex h-10 items-center gap-1 rounded-md border border-input bg-transparent px-3 text-sm',
                                    'focus-within:ring-2 focus-within:ring-ring focus-within:ring-offset-0',
                                )}
                            >
                                <span className="select-none text-muted-foreground">@</span>
                                <input
                                    {...register('username')}
                                    placeholder="username"
                                    autoCapitalize="none"
                                    autoCorrect="off"
                                    spellCheck={false}
                                    className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted-foreground"
                                />
                                <span className="select-none text-muted-foreground">:</span>
                                {editingInstance ? (
                                    <input
                                        {...register('instance')}
                                        autoFocus
                                        onBlur={() => setEditingInstance(false)}
                                        spellCheck={false}
                                        autoCapitalize="none"
                                        autoCorrect="off"
                                        className="w-44 bg-transparent text-right text-muted-foreground outline-none"
                                    />
                                ) : (
                                    <button
                                        type="button"
                                        onClick={() => setEditingInstance(true)}
                                        className="inline-flex items-center gap-1 text-muted-foreground transition-colors hover:text-foreground"
                                        title="Change instance"
                                    >
                                        {instance || GLOBAL_DOMAIN}
                                        <Pencil className="h-3 w-3"/>
                                    </button>
                                )}
                            </div>
                            {(errors.username || errors.instance) && (
                                <p className="text-xs text-destructive">
                                    {errors.username?.message ?? errors.instance?.message}
                                </p>
                            )}
                        </div>

                        <InstanceCorsWarning instance={instance}/>

                        <div className="space-y-1.5">
                            <Label htmlFor="display_name">Display name</Label>
                            <Input id="display_name" placeholder="Jane Doe" {...register('display_name')} />
                            {errors.display_name && <p className="text-xs text-destructive">{errors.display_name.message}</p>}
                        </div>

                        <div className="space-y-1.5">
                            <Label htmlFor="email">Email</Label>
                            <Input id="email" type="email" placeholder="jane@example.com" {...register('email')} />
                            {errors.email && <p className="text-xs text-destructive">{errors.email.message}</p>}
                        </div>

                        <div className="space-y-1.5">
                            <Label htmlFor="password">Password</Label>
                            <Input id="password" type="password" autoComplete="new-password" {...register('password')} />
                            {errors.password && <p className="text-xs text-destructive">{errors.password.message}</p>}
                        </div>

                        <Button type="submit" className="w-full" disabled={isSubmitting}>
                            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                            Create account
                        </Button>
                    </form>

                    <p className="mt-4 text-center text-sm text-muted-foreground">
                        Already have an account?{' '}
                        <Link to="/login" className="text-primary hover:underline">
                            Sign in
                        </Link>
                    </p>
                </CardContent>
            </Card>
        </div>
    )
}
