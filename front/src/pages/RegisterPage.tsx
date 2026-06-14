import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {Link, useNavigate} from 'react-router-dom'
import {Loader2} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from '@/components/ui/card'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {type RegisterForm, registerFormSchema} from '@/lib/schemas'
import {login, register as registerUser} from '@/api/auth'
import {apiErrorMessage} from '@/api/client'

export default function RegisterPage() {
    const navigate = useNavigate()

    const {
        register,
        handleSubmit,
        formState: {errors, isSubmitting},
    } = useForm<RegisterForm>({
        resolver: zodResolver(registerFormSchema),
        defaultValues: {username: '', display_name: '', email: '', password: ''},
    })

    const onSubmit = async (values: RegisterForm) => {
        try {
            await registerUser(values)
            await login(values.username, values.password, GLOBAL_DOMAIN)
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
                        Registering on <span className="font-medium text-foreground">{GLOBAL_DOMAIN}</span>.
                    </CardDescription>
                </CardHeader>
                <CardContent>
                    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4" noValidate>
                        <div className="space-y-1.5">
                            <Label htmlFor="username">Username</Label>
                            <Input
                                id="username"
                                placeholder="jane_doe"
                                autoCapitalize="none"
                                autoCorrect="off"
                                spellCheck={false}
                                {...register('username')}
                            />
                            {errors.username && <p className="text-xs text-destructive">{errors.username.message}</p>}
                        </div>

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
