import {Outlet} from 'react-router-dom'
import {TopBar} from './TopBar'
import {StatusBar} from './StatusBar'
import {UploadDialog} from '@/components/photos/UploadDialog'
import {useUploadStore} from '@/stores/upload'
import {useTagMetaQueue} from '@/hooks/useTags'
import {useSettings} from '@/hooks/useSettings'
import {SensitiveModeProvider} from '@/hooks/useSensitive'

/** App chrome for authenticated routes: global top bar + routed content + status bar. */
export function AppShell() {
    const {open, initialFiles, closeDialog} = useUploadStore()
    // Tag-preference writes are debounced, so the unload/visibility flush lives at app level (§4.1).
    useTagMetaQueue()
    // Blurred until the settings land, so a sensitive picture never flashes visible (feature 38 §3).
    const {data: settings} = useSettings()

    return (
        <SensitiveModeProvider value={settings?.sensitive_display_mode ?? 'blurred'}>
            {/* Below md: document scrolls (sticky header/footer) so mobile browser chrome can retract;
                md+: fixed viewport, content panes scroll internally instead (desktop has no chrome to hide). */}
            <div className="flex min-h-dvh flex-col bg-background text-foreground md:h-dvh md:overflow-hidden">
                <TopBar/>
                <main className="flex-1 md:min-h-0 md:overflow-hidden">
                    <Outlet/>
                </main>
                <StatusBar/>
                <UploadDialog
                    open={open}
                    onOpenChange={(o) => !o && closeDialog()}
                    initialFiles={initialFiles}
                />
            </div>
        </SensitiveModeProvider>
    )
}
