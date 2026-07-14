import {useEffect, useState} from 'react'
import {useForm} from 'react-hook-form'
import {zodResolver} from '@hookform/resolvers/zod'
import {Link, useNavigate, useSearchParams} from 'react-router-dom'
import {Loader2, Pencil, Ticket} from 'lucide-react'
import {toast} from 'sonner'
import {useQuery} from '@tanstack/react-query'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from '@/components/ui/card'
import {InstanceHealthWarning} from '@/components/common/InstanceHealthWarning'
import {cn} from '@/lib/utils'
import {getPreferredInstance, GLOBAL_DOMAIN, setPreferredInstance} from '@/lib/constants'
import {type RegisterForm, registerFormSchema} from '@/lib/schemas'
import {login, register as registerUser} from '@/api/auth'
import {getRegistrationInfo, previewInvite} from '@/api/invites'
import {apiErrorMessage} from '@/api/client'

/** Accept a pasted invite link or a grouped code (`ABC-DEF-GHI`) → the bare lowercase code. */
function normalizeInviteInput(s: string): string {
    const m = s.match(/invite=([A-Za-z0-9-]+)/)
    const raw = m ? m[1] : s
    return raw.replace(/[^A-Za-z0-9]/g, '').toLowerCase()
}

export default function RegisterPage() {
    const navigate = useNavigate()
    const [searchParams, setSearchParams] = useSearchParams()
    const inviteCode = searchParams.get('invite') ?? undefined

    const [editingInstance, setEditingInstance] = useState(false)
    const [codeInput, setCodeInput] = useState('')

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

    // Preview the invite so we can show "X invited you to join …" (best-effort — a bad code just
    // registers without provenance, the backend re-validates).
    const {data: invite} = useQuery({
        queryKey: ['invite-preview', inviteCode, instance],
        queryFn: () => previewInvite(inviteCode!, instance),
        enabled: !!inviteCode,
        retry: false,
    })

    // The instance's registration mode — to hide the form when it's invite-only and no valid invite.
    const {data: regInfo} = useQuery({
        queryKey: ['registration-info', instance],
        queryFn: () => getRegistrationInfo(instance),
        retry: false,
    })

    // Invite-only and no valid invite ⇒ the form can't succeed, so show a message instead.
    const inviteRequired = regInfo ? regInfo.mode !== 'open' : false
    const hasValidInvite = !!inviteCode && invite?.valid !== false
    const blocked = inviteRequired && !hasValidInvite

    // Persist the chosen instance so login and register stay in sync.
    useEffect(() => setPreferredInstance(instance), [instance])

    const onSubmit = async (values: RegisterForm) => {
        try {
            const {instance: domain, ...payload} = values
            await registerUser({...payload, invite_code: inviteCode}, domain)
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
                <CardContent className="space-y-4">
                    {/* Invite context: blocked instances get an explanatory message; otherwise a banner. */}
                    {blocked ? (
                        inviteCode ? (
                            // The link exists but isn't usable here (expired / used up / a tracking referral
                            // in an invite-only instance) → it's simply invalid.
                            <div
                                className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2.5 text-sm text-destructive">
                                <Ticket className="mt-0.5 h-4 w-4 shrink-0"/>
                                <span>This invite link is invalid or has expired. Ask whoever invited you for a fresh one, or switch to another instance.</span>
                            </div>
                        ) : (
                            <div
                                className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2.5 text-sm text-amber-700 dark:text-amber-500">
                                <Ticket className="mt-0.5 h-4 w-4 shrink-0"/>
                                <span>Registration on <span className="font-medium">{instance || GLOBAL_DOMAIN}</span> is invite-only. You need an invitation link, or switch to another instance.</span>
                            </div>
                        )
                    ) : (
                        inviteCode && (
                            <div className={cn(
                                'flex items-start gap-2 rounded-md border px-3 py-2 text-sm',
                                invite?.valid === false
                                    ? 'border-destructive/40 bg-destructive/10 text-destructive'
                                    : 'border-primary/30 bg-primary/10',
                            )}>
                                <Ticket className="mt-0.5 h-4 w-4 shrink-0"/>
                                {invite?.valid === false ? (
                                    <span>This invite link is invalid or has expired, but registration is open so you can still create an account.</span>
                                ) : invite?.invited_by ? (
                                    <span>
                                        <span className="font-medium">@{invite.invited_by}</span> invited you to join Archypix on{' '}
                                        <span className="font-medium">{instance || GLOBAL_DOMAIN}</span>. Create an account to get started.
                                    </span>
                                ) : (
                                    <span>You've been invited to join Archypix on <span
                                        className="font-medium">{instance || GLOBAL_DOMAIN}</span>.</span>
                                )}
                            </div>
                        )
                    )}

                    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4" noValidate>
                        {/* Handle field: @username:instance, instance editable on click — always shown so
                            the user can switch instance even when this one's registration is closed. */}
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

                        <InstanceHealthWarning instance={instance}/>

                        {!blocked && (
                            <>
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
                            </>
                        )}
                    </form>

                    {/* Blocked: let the user paste an invite link or type the code (ABC-DEF-GHI). */}
                    {blocked && (
                        <div className="space-y-2">
                            <Label htmlFor="invite-code">Have an invite?</Label>
                            <div className="flex gap-2">
                                <Input
                                    id="invite-code"
                                    value={codeInput}
                                    onChange={(e) => setCodeInput(e.target.value)}
                                    onKeyDown={(e) => {
                                        if (e.key === 'Enter') {
                                            e.preventDefault()
                                            const code = normalizeInviteInput(codeInput)
                                            if (code) setSearchParams({invite: code})
                                        }
                                    }}
                                    placeholder="ABC-DEF-GHI"
                                    autoCapitalize="characters"
                                    spellCheck={false}
                                    className="font-mono uppercase tracking-wider placeholder:tracking-wider"
                                />
                                <Button
                                    type="button"
                                    disabled={!normalizeInviteInput(codeInput)}
                                    onClick={() => {
                                        const code = normalizeInviteInput(codeInput)
                                        if (code) setSearchParams({invite: code})
                                    }}
                                >
                                    Continue
                                </Button>
                            </div>
                            <p className="text-xs text-muted-foreground">Paste the invite link or enter the code.</p>
                        </div>
                    )}

                    <p className="text-center text-sm text-muted-foreground">
                        Already have an account?{' '}
                        <Link to="/login" className="text-primary hover:underline">Sign in</Link>
                    </p>
                </CardContent>
            </Card>
        </div>
    )
}
