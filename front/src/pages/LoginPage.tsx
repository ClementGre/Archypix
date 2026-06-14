import {useState} from 'react'
import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {Link, useLocation, useNavigate} from 'react-router-dom'
import {Loader2, Pencil} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from '@/components/ui/card'
import {cn} from '@/lib/utils'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {type LoginForm, loginFormSchema} from '@/lib/schemas'
import {login} from '@/api/auth'
import {apiErrorMessage} from '@/api/client'

export default function LoginPage() {
    const navigate = useNavigate()
    const location = useLocation()
    const from = (location.state as { from?: { pathname: string } } | null)?.from?.pathname ?? '/'

    const [editingInstance, setEditingInstance] = useState(false)

    const {
        register,
        handleSubmit,
        watch,
        formState: {errors, isSubmitting},
    } = useForm<LoginForm>({
        resolver: zodResolver(loginFormSchema),
        defaultValues: {username: '', instance: GLOBAL_DOMAIN, password: ''},
    })

    const instance = watch('instance')

    const onSubmit = async (values: LoginForm) => {
        try {
            await login(values.username, values.password, values.instance)
            navigate(from, {replace: true})
        } catch (error) {
            toast.error('Login failed', {description: apiErrorMessage(error)})
        }
    }

    return (
        <div className="flex min-h-screen items-center justify-center bg-background p-6">
            <Card className="w-full max-w-md">
                <CardHeader className="space-y-1">
                    <CardTitle className="text-2xl">
                        <span className="text-primary">Archy</span>pix
                    </CardTitle>
                    <CardDescription>Sign in to your photo library.</CardDescription>
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

                        <div className="space-y-1.5">
                            <Label htmlFor="password">Password</Label>
                            <Input id="password" type="password" autoComplete="current-password" {...register('password')} />
                            {errors.password && <p className="text-xs text-destructive">{errors.password.message}</p>}
                        </div>

                        <Button type="submit" className="w-full" disabled={isSubmitting}>
                            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                            Sign in
                        </Button>
                    </form>

                    <p className="mt-4 text-center text-sm text-muted-foreground">
                        No account?{' '}
                        <Link to="/register" className="text-primary hover:underline">
                            Create one
                        </Link>
                    </p>
                </CardContent>
            </Card>
        </div>
    )
}
