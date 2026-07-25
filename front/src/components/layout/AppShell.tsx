import {Outlet} from 'react-router-dom'
import {TopBar} from './TopBar'
import {StatusBar} from './StatusBar'
import {UploadDialog} from '@/components/photos/UploadDialog'
import {useUploadStore} from '@/stores/upload'

/** App chrome for authenticated routes: global top bar + routed content + status bar. */
export function AppShell() {
    const {open, initialFiles, closeDialog} = useUploadStore()

    return (
        <div className="flex h-dvh flex-col overflow-hidden bg-background text-foreground">
            <TopBar/>
            <main className="min-h-0 flex-1 overflow-hidden">
                <Outlet/>
            </main>
            <StatusBar/>
            <UploadDialog
                open={open}
                onOpenChange={(o) => !o && closeDialog()}
                initialFiles={initialFiles}
            />
        </div>
    )
}
