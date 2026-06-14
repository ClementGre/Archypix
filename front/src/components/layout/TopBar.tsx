import {Link, NavLink, useLocation, useNavigate} from 'react-router-dom'
import {Images, LogOut, Moon, PanelLeft, PanelRight, Settings, Shield, Sun, Trash2, User as UserIcon, Wand2,} from 'lucide-react'
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
import {RowHeightSlider} from '@/components/photos/RowHeightSlider'
import {useAuthStore} from '@/stores/auth'
import {useThemeStore} from '@/stores/theme'
import {useUIStore} from '@/stores/ui'
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
    {to: '/tagging', label: 'Tagging pipeline', icon: Wand2},
    {to: '/trash', label: 'Trash', icon: Trash2},
    {to: '/settings', label: 'Settings', icon: Settings},
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
    const {leftSidebarOpen, rightSidebarOpen, toggleLeft, toggleRight} = useUIStore()
    const navigate = useNavigate()
    const isGallery = useLocation().pathname === '/'

    const items = NAV.filter((n) => !n.adminOnly || user?.is_admin)

    const handleLogout = async () => {
        await logout()
        navigate('/login', {replace: true})
    }

    return (
        <header className="flex h-14 shrink-0 items-center gap-1 border-b border-border bg-background px-3">
            {isGallery && (
                <Button
                    variant="ghost"
                    size="icon"
                    onClick={toggleLeft}
                    aria-label="Toggle tags panel"
                    className={cn(leftSidebarOpen && 'text-primary')}
                >
                    <PanelLeft className="h-4 w-4"/>
                </Button>
            )}

            <Link to="/" className="px-1.5 text-lg font-semibold tracking-tight">
                <span className="text-primary">Archy</span>pix
            </Link>

            <nav className="ml-1 flex items-center gap-0.5">
                {items.map(({to, label, icon: Icon, end}) => (
                    <NavLink
                        key={to}
                        to={to}
                        end={end}
                        title={label}
                        aria-label={label}
                        className={({isActive}) =>
                            cn(
                                'flex h-9 w-9 items-center justify-center rounded-md transition-colors',
                                isActive ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-muted hover:text-foreground',
                            )
                        }
                    >
                        <Icon className="h-4 w-4"/>
                    </NavLink>
                ))}
            </nav>

            <div className="ml-3 flex min-w-0 flex-1 items-center gap-2">{isGallery && <FilterControls/>}</div>

            <div className="flex shrink-0 items-center gap-1">
                {isGallery && <RowHeightSlider/>}
                {isGallery && (
                    <Button
                        variant="ghost"
                        size="icon"
                        onClick={toggleRight}
                        aria-label="Toggle details panel"
                        className={cn(rightSidebarOpen && 'text-primary')}
                    >
                        <PanelRight className="h-4 w-4"/>
                    </Button>
                )}

                <Button variant="ghost" size="icon" onClick={toggleTheme} aria-label="Toggle theme">
                    {theme === 'dark' ? <Sun className="h-4 w-4"/> : <Moon className="h-4 w-4"/>}
                </Button>

                <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                        <Button variant="ghost" className="h-9 gap-2 px-2">
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
