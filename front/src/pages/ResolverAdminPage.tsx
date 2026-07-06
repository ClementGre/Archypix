import {useEffect, useState} from 'react'
import {useNavigate} from 'react-router-dom'
import {useQueryClient} from '@tanstack/react-query'
import {ArrowUpRight, LogOut, Network, RefreshCw} from 'lucide-react'
import {Tabs, TabsContent, TabsList, TabsTrigger} from '@/components/ui/tabs'
import {Button} from '@/components/ui/button'
import {Switch} from '@/components/ui/switch'
import {ResolverLogin} from '@/components/resolver/ResolverLogin'
import {ResolverBackendsTab} from '@/components/resolver/ResolverBackendsTab'
import {ResolverUsersTab} from '@/components/resolver/ResolverUsersTab'
import {ResolverConfigMatrixTab} from '@/components/resolver/ResolverConfigMatrixTab'
import {ResolverSettingsTab} from '@/components/resolver/ResolverSettingsTab'
import {ResolverRoutinesTab} from '@/components/resolver/ResolverRoutinesTab'
import {ResolverInvitesTab} from '@/components/resolver/ResolverInvitesTab'
import {useResolverSession} from '@/hooks/useResolverAdmin'
import {useResolverAuthStore} from '@/stores/resolverAuth'
import {useAuthStore} from '@/stores/auth'
import {refresh as refreshSession} from '@/api/resolverAdmin'
import {GLOBAL_DOMAIN} from '@/lib/constants'

const AUTO_REFRESH_MS = 15_000
const TAB_LIST = 'inline-flex h-auto flex-wrap items-center justify-start gap-1 rounded-md bg-muted p-1'

/** Refresh the operator session shortly before it expires so a long-open tab never gets kicked out. */
function useSessionKeepAlive() {
    const expiresAt = useResolverAuthStore((s) => s.expiresAt)
    const refreshToken = useResolverAuthStore((s) => s.refreshToken)
    useEffect(() => {
        if (!expiresAt || !refreshToken) return
        const delay = Math.max(5_000, expiresAt - Date.now() - 60_000)
        const t = setTimeout(async () => {
            try {
                const s = await refreshSession(refreshToken)
                useResolverAuthStore.getState().setSession({
                    sessionToken: s.session_token,
                    refreshToken: s.refresh_token,
                    expiresInSecs: s.expires_in_secs,
                })
            } catch {
                // The 401 interceptor handles a hard expiry; nothing to do here.
            }
        }, delay)
        return () => clearTimeout(t)
    }, [expiresAt, refreshToken])
}

export default function ResolverAdminPage() {
    const session = useResolverSession()
    useSessionKeepAlive()
    const clear = useResolverAuthStore((s) => s.clear)
    const queryClient = useQueryClient()
    const navigate = useNavigate()
    const user = useAuthStore((s) => s.user)
    const instance = useAuthStore((s) => s.instance)
    const [autoRefresh, setAutoRefresh] = useState(true)

    if (!session) return <ResolverLogin/>

    const interval: number | false = autoRefresh ? AUTO_REFRESH_MS : false

    const signOut = () => {
        clear()
        queryClient.removeQueries({queryKey: ['resolverAdmin']})
    }

    return (
        <div className="h-full min-h-screen overflow-y-auto bg-background px-6 py-4">
            <div className="mx-auto max-w-6xl space-y-4">
                {/* Compact header */}
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                    <h1 className="flex items-center gap-1.5 text-base font-semibold">
                        <Network className="h-4 w-4 text-primary"/>
                        Fleet
                        <span className="text-xs font-normal text-muted-foreground">{GLOBAL_DOMAIN}</span>
                    </h1>
                    <label className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
                        <Switch checked={autoRefresh} onCheckedChange={setAutoRefresh}/>
                        Auto-refresh
                    </label>
                    <Button variant="ghost" size="icon" className="h-8 w-8" title="Refresh (clear cache)" aria-label="Refresh"
                            onClick={() => queryClient.invalidateQueries({queryKey: ['resolverAdmin']})}>
                        <RefreshCw className="h-4 w-4"/>
                    </Button>
                    {user?.is_admin && (
                        <Button variant="outline" size="sm" className="h-8 gap-1.5" onClick={() => navigate('/admin')}
                                title={`Back to ${instance ?? 'your'} admin dashboard`}>
                            <ArrowUpRight className="h-3.5 w-3.5"/>
                            {instance ?? 'Admin'}
                        </Button>
                    )}
                    <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-destructive" onClick={signOut}>
                        <LogOut className="h-4 w-4"/>
                        Sign out
                    </Button>
                </div>

                <Tabs defaultValue="backends">
                    <TabsList className={TAB_LIST}>
                        <TabsTrigger value="backends">Backends</TabsTrigger>
                        <TabsTrigger value="users">Users</TabsTrigger>
                        <TabsTrigger value="matrix">Config matrix</TabsTrigger>
                        <TabsTrigger value="settings">Settings</TabsTrigger>
                        <TabsTrigger value="routines">Routines</TabsTrigger>
                        <TabsTrigger value="invites">Invites</TabsTrigger>
                    </TabsList>
                    <TabsContent value="backends" className="mt-6"><ResolverBackendsTab refetchInterval={interval}/></TabsContent>
                    <TabsContent value="users" className="mt-6"><ResolverUsersTab refetchInterval={interval}/></TabsContent>
                    <TabsContent value="matrix" className="mt-6"><ResolverConfigMatrixTab/></TabsContent>
                    <TabsContent value="settings" className="mt-6"><ResolverSettingsTab/></TabsContent>
                    <TabsContent value="routines" className="mt-6"><ResolverRoutinesTab refetchInterval={interval}/></TabsContent>
                    <TabsContent value="invites" className="mt-6"><ResolverInvitesTab/></TabsContent>
                </Tabs>
            </div>
        </div>
    )
}
