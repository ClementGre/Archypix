import {Outlet} from 'react-router-dom'
import {TopBar} from './TopBar'

/** App chrome for authenticated routes: global top bar + routed content. */
export function AppShell() {
    return (
        <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
            <TopBar/>
            <main className="min-h-0 flex-1 overflow-hidden">
                <Outlet/>
            </main>
        </div>
    )
}
