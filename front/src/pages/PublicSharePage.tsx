import {useEffect, useMemo, useState} from 'react'
import {useParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {Loader2, Lock} from 'lucide-react'
import type {PublicShareMeta} from '@/api/publicShares'
import {getPublicMeta, resolvePublicBackend, unlockPublicShare} from '@/api/publicShares'
import {apiErrorMessage} from '@/api/client'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {PublicTopBar} from '@/components/public/PublicTopBar'
import {PublicGallery} from '@/components/public/PublicGallery'
import {PublicDetailPanel} from '@/components/public/PublicDetailPanel'
import {PublicStatusBar} from '@/components/public/PublicStatusBar'
import type {PublicShareContext} from '@/components/public/context'
import {PublicShareProvider} from '@/components/public/context'
import {PictureSourceProvider} from '@/components/photos/pictureSource'
import {usePublicPictureSource} from '@/components/public/publicPictureSource'
import {SidePanel} from '@/components/layout/SidePanel'
import {useUIStore} from '@/stores/ui'
import {useSelectionStore} from '@/stores/selection'
import {useIsMobile} from '@/hooks/useMediaQuery'

/** Keep every public share page out of search indexes (§13). */
function useNoIndex() {
    useEffect(() => {
        const tag = document.createElement('meta')
        tag.name = 'robots'
        tag.content = 'noindex, nofollow'
        document.head.appendChild(tag)
        return () => {
            document.head.removeChild(tag)
        }
    }, [])
}

export default function PublicSharePage() {
    useNoIndex()
    const {globalDomain = '', username = '', token = ''} = useParams()
    const [session, setSession] = useState<string | null>(() => sessionStorage.getItem(`pubshare:${token}`))

    const backendQ = useQuery({
        queryKey: ['publicBackend', globalDomain, username],
        queryFn: () => resolvePublicBackend(username, globalDomain),
        retry: false,
    })
    const backendUrl = backendQ.data
    const metaQ = useQuery({
        queryKey: ['publicMeta', backendUrl, token],
        queryFn: () => getPublicMeta(backendUrl!, token),
        enabled: !!backendUrl,
        retry: false,
    })

    if (backendQ.isPending || (backendUrl && metaQ.isPending)) {
        return <Centered><Loader2 className="h-6 w-6 animate-spin text-muted-foreground"/></Centered>
    }
    if (backendQ.isError) {
        return <ErrorCard title="Share unavailable" message={apiErrorMessage(backendQ.error)}/>
    }
    if (metaQ.isError || !metaQ.data || !backendUrl) {
        return (
            <ErrorCard
                title="This link is invalid or has been revoked"
                message="Ask the owner for a fresh link."
            />
        )
    }
    const meta = metaQ.data

    if (meta.requires_password && !session) {
        return (
            <PasswordGate
                meta={meta}
                onUnlock={async (password) => {
                    const jwt = await unlockPublicShare(backendUrl, token, password)
                    sessionStorage.setItem(`pubshare:${token}`, jwt)
                    setSession(jwt)
                }}
            />
        )
    }

    return (
        <UnlockedShare
            backendUrl={backendUrl}
            token={token}
            ownerUsername={username}
            globalDomain={globalDomain}
            meta={meta}
            session={session}
            onSessionExpired={() => {
                sessionStorage.removeItem(`pubshare:${token}`)
                setSession(null)
            }}
        />
    )
}

function UnlockedShare(props: {
    backendUrl: string
    token: string
    ownerUsername: string
    globalDomain: string
    meta: PublicShareMeta
    session: string | null
    onSessionExpired: () => void
}) {
    const rightSidebarOpen = useUIStore((s) => s.rightSidebarOpen)
    const rightSidebarWidth = useUIStore((s) => s.rightSidebarWidth)
    const setRightWidth = useUIStore((s) => s.setRightWidth)
    const mobileDrawer = useUIStore((s) => s.mobileDrawer)
    const closeMobileDrawer = useUIStore((s) => s.closeMobileDrawer)
    const isMobile = useIsMobile()
    const rightOpen = isMobile ? mobileDrawer === 'right' : rightSidebarOpen

    // The public page shares the global feature-14 selection store with the app; clear it on
    // enter/leave so a selection never bleeds between the app and a public album (different id spaces).
    const clearSelection = useSelectionStore((s) => s.clear)
    useEffect(() => {
        clearSelection()
        return clearSelection
    }, [clearSelection])

    const ctx: PublicShareContext = useMemo(
        () => ({
            backendUrl: props.backendUrl,
            token: props.token,
            ownerUsername: props.ownerUsername,
            globalDomain: props.globalDomain,
            meta: props.meta,
            session: props.session,
        }),
        [props.backendUrl, props.token, props.ownerUsername, props.globalDomain, props.meta, props.session],
    )

    const pictureSource = usePublicPictureSource({
        backendUrl: props.backendUrl,
        token: props.token,
        session: props.session,
        canDownload: props.meta.permissions.allow_originals,
    })

    return (
        <PublicShareProvider value={ctx}>
            <PictureSourceProvider value={pictureSource}>
                <div className="flex min-h-dvh flex-col bg-background text-foreground md:h-dvh">
                    <PublicTopBar/>
                    <div className="flex min-h-0 flex-1">
                        <main className="min-w-0 flex-1 md:overflow-y-auto">
                            <PublicGallery/>
                        </main>
                        {/* Resizable, mobile-drawer details panel — the shared workspace SidePanel. */}
                        <SidePanel
                            side="right"
                            width={rightSidebarWidth}
                            onResize={setRightWidth}
                            open={rightOpen}
                            onClose={closeMobileDrawer}
                        >
                            <div className="h-full overflow-y-auto">
                                <PublicDetailPanel/>
                            </div>
                        </SidePanel>
                    </div>
                    <PublicStatusBar/>
                </div>
            </PictureSourceProvider>
        </PublicShareProvider>
    )
}

function PasswordGate({meta, onUnlock}: { meta: PublicShareMeta; onUnlock: (pw: string) => Promise<void> }) {
    const [password, setPassword] = useState('')
    const [error, setError] = useState<string | null>(null)
    const [busy, setBusy] = useState(false)

    const submit = async (e: React.FormEvent) => {
        e.preventDefault()
        setBusy(true)
        setError(null)
        try {
            await onUnlock(password)
        } catch (err) {
            setError(apiErrorMessage(err, 'Incorrect password'))
        } finally {
            setBusy(false)
        }
    }

    return (
        <Centered>
            <form onSubmit={submit} className="w-full max-w-sm rounded-lg border border-border bg-card p-6 shadow-sm">
                <div className="mb-4 flex items-center gap-2">
                    <Lock className="h-5 w-5 text-muted-foreground"/>
                    <h1 className="text-lg font-semibold">{meta.name}</h1>
                </div>
                <p className="mb-4 text-sm text-muted-foreground">This share is password-protected.</p>
                <Input
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    placeholder="Password"
                    autoFocus
                />
                {error && <p className="mt-2 text-sm text-destructive">{error}</p>}
                <Button type="submit" className="mt-4 w-full" disabled={busy || !password}>
                    {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin"/>}
                    Unlock
                </Button>
            </form>
        </Centered>
    )
}

function Centered({children}: { children: React.ReactNode }) {
    return <div className="flex h-dvh items-center justify-center bg-background p-4 text-foreground">{children}</div>
}

function ErrorCard({title, message}: { title: string; message: string }) {
    return (
        <Centered>
            <div className="w-full max-w-md rounded-lg border border-border bg-card p-6 text-center shadow-sm">
                <h1 className="text-lg font-semibold">{title}</h1>
                <p className="mt-2 text-sm text-muted-foreground">{message}</p>
            </div>
        </Centered>
    )
}
