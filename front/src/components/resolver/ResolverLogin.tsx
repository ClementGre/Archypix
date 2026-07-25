import {useState} from 'react'
import {KeyRound, Loader2, Network} from 'lucide-react'
import {toast} from 'sonner'
import {Card, CardContent, CardDescription, CardHeader, CardTitle} from '@/components/ui/card'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Button} from '@/components/ui/button'
import {InstanceHealthWarning} from '@/components/common/InstanceHealthWarning'
import {login} from '@/api/resolverAdmin'
import {getResolverInfo} from '@/api/resolve'
import {apiErrorMessage} from '@/api/client'
import {useResolverAuthStore} from '@/stores/resolverAuth'
import {useAuthStore} from '@/stores/auth'
import {GLOBAL_DOMAIN} from '@/lib/constants'

/**
 * Operator-token login for the fleet dashboard (feature 24 §3). The token *is* the credential.
 * The operator can target a resolver other than the default global domain (feature 25): the domain is
 * bootstrapped via `/archypix-resolver/info` to find its `api_url` before exchanging the token.
 */
export function ResolverLogin() {
    // Default to the logged-in user's instance (their global domain fronts its resolver), else the
    // configured global domain.
    const userInstance = useAuthStore((s) => s.instance)
    const [domain, setDomain] = useState(userInstance ?? GLOBAL_DOMAIN)
    const [token, setToken] = useState('')
    const [busy, setBusy] = useState(false)
    const setResolverUrl = useResolverAuthStore((s) => s.setResolverUrl)
    const setSession = useResolverAuthStore((s) => s.setSession)

    const submit = async (e: React.FormEvent) => {
        e.preventDefault()
        if (!token.trim() || !domain.trim()) return
        setBusy(true)
        try {
            const info = await getResolverInfo(domain.trim())
            if (!info.is_resolver) {
                toast.error('No fleet dashboard here', {
                    description: `${domain.trim()} is a standalone instance, not a resolver.`,
                })
                return
            }
            setResolverUrl(info.api_url)
            const s = await login(token.trim())
            setSession({sessionToken: s.session_token, refreshToken: s.refresh_token, expiresInSecs: s.expires_in_secs})
            toast.success('Signed in to the fleet dashboard')
        } catch (e) {
            toast.error('Login failed', {description: apiErrorMessage(e)})
        } finally {
            setBusy(false)
        }
    }

    return (
        <div className="flex min-h-dvh items-center justify-center bg-background p-6">
            <Card className="w-full max-w-md">
                <CardHeader className="space-y-1">
                    <CardTitle className="flex items-center gap-2 text-2xl">
                        <Network className="h-6 w-6 text-primary"/>
                        Fleet dashboard
                    </CardTitle>
                    <CardDescription>
                        Resolver operator console. Choose a resolver and sign in with its operator token.
                    </CardDescription>
                </CardHeader>
                <CardContent>
                    <form onSubmit={submit} className="space-y-4">
                        <div className="space-y-1.5">
                            <Label htmlFor="resolver-domain">Resolver domain</Label>
                            <Input
                                id="resolver-domain"
                                autoComplete="off"
                                autoCapitalize="none"
                                autoCorrect="off"
                                spellCheck={false}
                                value={domain}
                                onChange={(e) => setDomain(e.target.value)}
                                placeholder={GLOBAL_DOMAIN}
                            />
                        </div>

                        <InstanceHealthWarning instance={domain}/>

                        <div className="space-y-1.5">
                            <Label htmlFor="operator-token">Operator token</Label>
                            <div className="relative">
                                <KeyRound className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"/>
                                <Input
                                    id="operator-token"
                                    type="password"
                                    autoComplete="off"
                                    value={token}
                                    onChange={(e) => setToken(e.target.value)}
                                    placeholder="paste the operator token"
                                    className="pl-8 font-mono"
                                />
                            </div>
                        </div>
                        <Button type="submit" className="w-full" disabled={busy || !token.trim() || !domain.trim()}>
                            {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                            Sign in
                        </Button>
                    </form>
                </CardContent>
            </Card>
        </div>
    )
}
