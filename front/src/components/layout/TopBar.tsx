import {Link, NavLink, useLocation, useNavigate} from 'react-router-dom'
import {useQueryClient} from '@tanstack/react-query'
import {Images, LogOut, Moon, PanelLeft, PanelRight, RefreshCw, Settings, Shield, Sun, Trash2, Upload, User as UserIcon, Wand2,} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Avatar, AvatarFallback} from '@/components/ui/avatar'
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuLabel,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {FilterControls} from '@/components/photos/FilterControls'
import {useAuthStore} from '@/stores/auth'
import {useThemeStore} from '@/stores/theme'
import {useUIStore} from '@/stores/ui'
import {useUploadStore} from '@/stores/upload'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {logout} from '@/api/auth'
import {cn} from '@/lib/utils'

interface NavItem {
    to: string
    label: string
    icon: typeof Images
    end?: boolean
    adminOnly?: boolean
}

const NAV: NavItem[] = [
    {to: '/', label: 'Gallery', icon: Images, end: true},
    {to: '/tagging', label: 'Tagging services', icon: Wand2},
    {to: '/trash', label: 'Trash', icon: Trash2},
    {to: '/admin', label: 'Admin', icon: Shield, adminOnly: true},
]

function initials(name: string): string {
    return name
        .split(/\s+/)
        .filter(Boolean)
        .slice(0, 2)
        .map((w) => w[0]?.toUpperCase())
        .join('')
}

/** Single unified top bar: nav + sidebar toggles + gallery search/filters + user. */
export function TopBar() {
    const user = useAuthStore((s) => s.user)
    const instance = useAuthStore((s) => s.instance)
    const theme = useThemeStore((s) => s.theme)
    const toggleTheme = useThemeStore((s) => s.toggle)
    const {leftSidebarOpen, rightSidebarOpen, mobileDrawer, toggleLeft, toggleRight, toggleMobileDrawer} = useUIStore()
    const openUpload = useUploadStore((s) => s.openDialog)
    const navigate = useNavigate()
    const queryClient = useQueryClient()
    const isMobile = useIsMobile()
    const isGallery = useLocation().pathname === '/'

    // Manual catch-all refresh — for the residual cases proper invalidation can't cover
    // (asynchronous pipeline re-tagging / federated share delivery that lands after the settle pass).
    const refreshAll = () =>
        ['pictures', 'tags', 'tagging', 'shares', 'hierarchies'].forEach((key) =>
            queryClient.invalidateQueries({queryKey: [key]}),
        )

    const items = NAV.filter((n) => !n.adminOnly || user?.is_admin)

    // On mobile the toggles drive the overlay drawers; on desktop the docked panels.
    const onToggleLeft = () => (isMobile ? toggleMobileDrawer('left') : toggleLeft())
    const onToggleRight = () => (isMobile ? toggleMobileDrawer('right') : toggleRight())
    const leftActive = isMobile ? mobileDrawer === 'left' : leftSidebarOpen
    const rightActive = isMobile ? mobileDrawer === 'right' : rightSidebarOpen

    const handleLogout = async () => {
        await logout()
        navigate('/login', {replace: true})
    }

    return (
        <header className="flex h-12 shrink-0 items-center gap-1 border-b border-border bg-background px-2 sm:px-3">
            {isGallery && (
                <Button
                    variant="ghost"
                    size="icon"
                    onClick={onToggleLeft}
                    aria-label="Toggle tags panel"
                    className={cn('shrink-0', leftActive && 'text-primary')}
                >
                    <PanelLeft className="h-4 w-4"/>
                </Button>
            )}

            <Link to="/" className="shrink-0 px-1.5 text-lg font-semibold tracking-tight">
                <span className="text-primary">Archy</span>pix
            </Link>

            {/* Primary nav — collapses into the user menu on mobile. */}
            <nav className="ml-1 hidden items-center gap-0.5 md:flex">
                {items.map(({to, label, icon: Icon, end}) => (
                    <NavLink
                        key={to}
                        to={to}
                        end={end}
                        title={label}
                        aria-label={label}
                        className={({isActive}) =>
                            cn(
                                'flex h-8 w-8 items-center justify-center rounded-md transition-colors',
                                isActive ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-muted hover:text-foreground',
                            )
                        }
                    >
                        <Icon className="h-4 w-4"/>
                    </NavLink>
                ))}
            </nav>

            <div className="ml-2 flex min-w-0 flex-1 items-center gap-2 sm:ml-3">{isGallery && <FilterControls/>}</div>

            <div className="flex shrink-0 items-center gap-0.5 sm:gap-1">
                {isGallery && (
                    <Button
                        variant="ghost"
                        size="icon"
                        onClick={refreshAll}
                        title="Refresh"
                        aria-label="Refresh"
                        className="text-muted-foreground hover:text-foreground"
                    >
                        <RefreshCw className="h-4 w-4"/>
                    </Button>
                )}
                {isGallery && (
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => openUpload()}
                        className="gap-1.5 text-muted-foreground hover:text-foreground"
                    >
                        <Upload className="h-4 w-4"/>
                        <span className="hidden sm:inline">Upload</span>
                    </Button>
                )}
                {/* On mobile a single tap opens the details drawer and multi-select uses the
                    floating bar, so this toggle is only useful on desktop. */}
                {isGallery && !isMobile && (
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

                {/* Theme toggle moves into the user menu on mobile to keep the bar minimal. */}
                {!isMobile && (
                    <Button variant="ghost" size="icon" onClick={toggleTheme} aria-label="Toggle theme">
                        {theme === 'dark' ? <Sun className="h-4 w-4"/> : <Moon className="h-4 w-4"/>}
                    </Button>
                )}

                <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                        <Button variant="ghost" className="h-8 gap-2 px-1.5 sm:px-2">
                            <Avatar className="h-7 w-7">
                                <AvatarFallback className="bg-primary/15 text-xs text-primary">
                                    {user ? initials(user.display_name || user.username) : <UserIcon className="h-4 w-4"/>}
                                </AvatarFallback>
                            </Avatar>
                            <span className="hidden text-sm font-medium lg:inline">{user?.display_name ?? user?.username}</span>
                        </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-56">
                        <DropdownMenuLabel className="flex flex-col gap-0.5">
                            <span>{user?.display_name}</span>
                            <span className="text-xs font-normal text-muted-foreground">
                @{user?.username}
                                {instance ? `:${instance}` : ''}
              </span>
                        </DropdownMenuLabel>
                        <DropdownMenuSeparator/>

                        {/* Mobile-only nav + theme toggle (hidden once the in-bar controls are visible). */}
                        <div className="md:hidden">
                            {items.map(({to, label, icon: Icon}) => (
                                <DropdownMenuItem key={to} onClick={() => navigate(to)}>
                                    <Icon className="mr-2 h-4 w-4"/>
                                    {label}
                                </DropdownMenuItem>
                            ))}
                            <DropdownMenuSeparator/>
                            <DropdownMenuItem onClick={toggleTheme}>
                                {theme === 'dark' ? <Sun className="mr-2 h-4 w-4"/> : <Moon className="mr-2 h-4 w-4"/>}
                                {theme === 'dark' ? 'Light mode' : 'Dark mode'}
                            </DropdownMenuItem>
                            <DropdownMenuSeparator/>
                        </div>

                        <DropdownMenuItem onClick={() => navigate('/settings')}>
                            <Settings className="mr-2 h-4 w-4"/>
                            Settings
                        </DropdownMenuItem>
                        <DropdownMenuSeparator/>
                        <DropdownMenuItem onClick={handleLogout} className="text-destructive focus:text-destructive">
                            <LogOut className="mr-2 h-4 w-4"/>
                            Log out
                        </DropdownMenuItem>
                    </DropdownMenuContent>
                </DropdownMenu>
            </div>
        </header>
    )
}
