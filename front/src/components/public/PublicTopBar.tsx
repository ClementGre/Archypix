import {useMemo, useState} from 'react'
import {Link, useNavigate} from 'react-router-dom'
import {useQueryClient} from '@tanstack/react-query'
import {BookmarkPlus, Check, Copy, LogIn, LogOut, Moon, Network, PanelRight, Shield, Sun, Upload, User as UserIcon, UserPlus} from 'lucide-react'
import {toast} from 'sonner'
import {logout} from '@/api/auth'
import {saveCopyFromPublic, subscribeToPublic} from '@/api/publicShares'
import {apiErrorMessage} from '@/api/client'
import {GLOBAL_DOMAIN} from '@/lib/constants'
import {useAuthStore} from '@/stores/auth'
import {useThemeStore} from '@/stores/theme'
import {useUIStore} from '@/stores/ui'
import {useSelectionStore} from '@/stores/selection'
import {useIncomingShares} from '@/hooks/useShares'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {cn} from '@/lib/utils'
import {Button} from '@/components/ui/button'
import {Logo} from '@/components/common/Logo'
import {Avatar, AvatarFallback} from '@/components/ui/avatar'
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuLabel,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {UploadDialog} from '@/components/photos/UploadDialog'
import {usePublicShare} from '@/components/public/context'
import {usePublicUploadSource} from '@/components/public/publicUploadSource'

/**
 * Top bar for the public share page: mirrors the app `TopBar` chrome (brand, theme toggle, details-panel
 * toggle, a full user menu / sign-in + create-account) using the same shared primitives, with the share
 * info standing in for the breadcrumb. The visitor's Convert/Upload actions are hidden from the album owner.
 */
export function PublicTopBar() {
    const {meta, ownerUsername, globalDomain, token} = usePublicShare()
    const selectedIds = useSelectionStore((s) => s.includeIds)
    const user = useAuthStore((s) => s.user)
    const instance = useAuthStore((s) => s.instance)
    const isResolver = useAuthStore((s) => s.isResolver)
    const theme = useThemeStore((s) => s.theme)
    const toggleTheme = useThemeStore((s) => s.toggle)
    const {rightSidebarOpen, mobileDrawer, toggleRight, toggleMobileDrawer} = useUIStore()
    const isMobile = useIsMobile()
    const navigate = useNavigate()
    const queryClient = useQueryClient()
    const uploadSource = usePublicUploadSource()
    const [uploadOpen, setUploadOpen] = useState(false)
    const [busy, setBusy] = useState(false)
    const perms = meta.permissions

    // The album's own owner can't convert/save/upload their own pictures (the backend rejects it too).
    const isOwner = !!user && user.username === ownerUsername && (instance || GLOBAL_DOMAIN) === globalDomain

    // Already converted? A live incoming share from this owner covering this album's tag means Convert
    // would just duplicate it — offer a note instead. (`shared_tag_path` is `SharedToMe.<handle>.<tag>`.)
    const {data: incoming} = useIncomingShares(!!user)
    const alreadySubscribed = useMemo(
        () =>
            (incoming ?? []).some(
                (s) =>
                    (s.status === 'active' || s.status === 'pending') &&
                    s.sender_username === ownerUsername &&
                    s.sender_instance === globalDomain &&
                    s.shared_tag_path != null &&
                    s.shared_tag_path.split('.').slice(2).join('.') === meta.tag_path,
            ),
        [incoming, ownerUsername, globalDomain, meta.tag_path],
    )

    const onToggleRight = () => (isMobile ? toggleMobileDrawer('right') : toggleRight())
    const rightActive = isMobile ? mobileDrawer === 'right' : rightSidebarOpen

    const convert = async () => {
        setBusy(true)
        try {
            await subscribeToPublic({owner_username: ownerUsername, owner_instance: globalDomain, token})
            toast.success('Added to your account. You now have a new incoming share.')
        } catch (e) {
            toast.error(apiErrorMessage(e))
        } finally {
            setBusy(false)
        }
    }

    const saveSelected = async () => {
        const ids = [...selectedIds]
        if (ids.length === 0) {
            toast.info('Select some photos first.')
            return
        }
        setBusy(true)
        let ok = 0
        for (const id of ids) {
            try {
                await saveCopyFromPublic({owner_username: ownerUsername, owner_instance: globalDomain, token, picture_id: id})
                ok++
            } catch {
                /* keep going */
            }
        }
        setBusy(false)
        toast.success(`Saved ${ok} of ${ids.length} to your library.`)
    }

    const handleLogout = async () => {
        await logout()
        queryClient.clear()
        // Stay on the public share (it needs no auth); just drop the session.
    }

    return (
        <header className="sticky top-0 z-10 flex shrink-0 flex-wrap items-center gap-1 border-b border-border bg-background px-2 py-1.5 sm:px-3">
            <Link to="/" aria-label="Archypix home" className="shrink-0">
                <Logo/>
            </Link>

            {/* Share info stands in for the gallery breadcrumb (title + owner; description on hover). */}
            <div className="min-w-0 flex-1">
                <h1 className="truncate text-sm font-semibold leading-tight" title={meta.message ?? meta.name}>{meta.name}</h1>
                <p className="truncate text-xs leading-tight text-muted-foreground">
                    {meta.owner_display} · {meta.picture_count} photo{meta.picture_count === 1 ? '' : 's'}
                    {meta.view_only && ' · view-only'}
                    {selectedIds.length > 0 && ` · ${selectedIds.length} selected`}
                </p>
            </div>

            <div className="flex shrink-0 flex-wrap items-center gap-1 sm:gap-2">
                {perms.allow_upload && !isOwner && (
                    <Button size="sm" variant="secondary" onClick={() => setUploadOpen(true)}>
                        <Upload className="mr-1.5 h-4 w-4"/> Upload
                    </Button>
                )}

                {perms.allow_originals && !isOwner && user && (
                    <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                            <Button size="sm" disabled={busy}>
                                <BookmarkPlus className="mr-1.5 h-4 w-4"/> Add to my account
                            </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-64">
                            <DropdownMenuItem onClick={saveSelected}>
                                <Copy className="mr-2 h-4 w-4"/>
                                Save selected copies ({selectedIds.length})
                            </DropdownMenuItem>
                            {alreadySubscribed ? (
                                <DropdownMenuItem disabled>
                                    <Check className="mr-2 h-4 w-4 text-emerald-500"/>
                                    Already in your incoming shares
                                </DropdownMenuItem>
                            ) : (
                                <DropdownMenuItem onClick={convert}>
                                    <BookmarkPlus className="mr-2 h-4 w-4"/>
                                    Convert to a share on your account
                                </DropdownMenuItem>
                            )}
                        </DropdownMenuContent>
                    </DropdownMenu>
                )}

                {/* Details-panel toggle (desktop) — same affordance as the app bar. */}
                {!isMobile && (
                    <Button
                        variant="ghost"
                        size="icon"
                        onClick={onToggleRight}
                        aria-label="Toggle details panel"
                        className={cn(rightActive && 'text-primary')}
                    >
                        <PanelRight className="h-4 w-4"/>
                    </Button>
                )}
                {!isMobile && (
                    <Button variant="ghost" size="icon" onClick={toggleTheme} aria-label="Toggle theme">
                        {theme === 'dark' ? <Sun className="h-4 w-4"/> : <Moon className="h-4 w-4"/>}
                    </Button>
                )}

                {user ? (
                    <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                            <Button variant="ghost" className="h-8 gap-2 px-1.5 sm:px-2">
                                <Avatar className="h-7 w-7">
                                    <AvatarFallback className="bg-primary/15 text-xs text-primary">
                                        {(user.display_name || user.username).slice(0, 2).toUpperCase()}
                                    </AvatarFallback>
                                </Avatar>
                                <span className="hidden text-sm font-medium lg:inline">{user.display_name ?? user.username}</span>
                            </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-56">
                            <DropdownMenuLabel className="flex flex-col gap-0.5">
                                <span>{user.display_name}</span>
                                <span className="text-xs font-normal text-muted-foreground">
                                    @{user.username}{instance ? `:${instance}` : ''}
                                </span>
                            </DropdownMenuLabel>
                            <DropdownMenuSeparator/>
                            <DropdownMenuItem onClick={() => navigate('/')}>
                                <UserIcon className="mr-2 h-4 w-4"/>
                                Go to my gallery
                            </DropdownMenuItem>
                            <DropdownMenuItem onClick={() => navigate('/settings')}>
                                <UserIcon className="mr-2 h-4 w-4"/>
                                Profile
                            </DropdownMenuItem>
                            {user.is_admin && (
                                <>
                                    <DropdownMenuItem onClick={() => navigate('/admin')}>
                                        <Shield className="mr-2 h-4 w-4"/>
                                        Admin
                                    </DropdownMenuItem>
                                    {isResolver && (
                                        <DropdownMenuItem onClick={() => navigate('/admin/resolver')}>
                                            <Network className="mr-2 h-4 w-4"/>
                                            Fleet dashboard
                                        </DropdownMenuItem>
                                    )}
                                </>
                            )}
                            {isMobile && (
                                <DropdownMenuItem onClick={toggleTheme}>
                                    {theme === 'dark' ? <Sun className="mr-2 h-4 w-4"/> : <Moon className="mr-2 h-4 w-4"/>}
                                    {theme === 'dark' ? 'Light mode' : 'Dark mode'}
                                </DropdownMenuItem>
                            )}
                            <DropdownMenuSeparator/>
                            <DropdownMenuItem onClick={handleLogout} className="text-destructive focus:text-destructive">
                                <LogOut className="mr-2 h-4 w-4"/>
                                Log out
                            </DropdownMenuItem>
                        </DropdownMenuContent>
                    </DropdownMenu>
                ) : (
                    <>
                        <Button size="sm" variant="ghost" asChild>
                            <Link to="/login">
                                <LogIn className="mr-1.5 h-4 w-4"/> Sign in
                            </Link>
                        </Button>
                        <Button size="sm" variant="outline" asChild>
                            <Link to="/register">
                                <UserPlus className="mr-1.5 h-4 w-4"/> Create account
                            </Link>
                        </Button>
                    </>
                )}
            </div>

            {/* Reuse the shared upload dialog via a token-gated public UploadSource. */}
            <UploadDialog open={uploadOpen} onOpenChange={setUploadOpen} source={uploadSource}/>
        </header>
    )
}
