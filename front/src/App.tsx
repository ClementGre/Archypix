import {QueryClient, QueryClientProvider} from '@tanstack/react-query'
import {BrowserRouter, Navigate, Route, Routes} from 'react-router-dom'
import {Toaster} from '@/components/ui/sonner'
import {ProtectedRoute} from '@/components/layout/ProtectedRoute'
import {AppShell} from '@/components/layout/AppShell'
import LoginPage from '@/pages/LoginPage'
import RegisterPage from '@/pages/RegisterPage'
import GalleryPage from '@/pages/GalleryPage'
import PhotoPage from '@/pages/PhotoPage'
import TagsPage from '@/pages/TagsPage'
import TaggingPage from '@/pages/TaggingPage'
import ServiceEditorPage from '@/pages/ServiceEditorPage'
import SharesPage from '@/pages/SharesPage'
import SettingsPage from '@/pages/SettingsPage'
import TrashPage from '@/pages/TrashPage'
import AdminPage from '@/pages/AdminPage'

const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            staleTime: 30_000,
            retry: 1,
            refetchOnWindowFocus: false,
        },
    },
})

export default function App() {
    return (
        <QueryClientProvider client={queryClient}>
            <BrowserRouter>
                <Routes>
                    <Route path="/login" element={<LoginPage/>}/>
                    <Route path="/register" element={<RegisterPage/>}/>

                    {/* Authenticated app */}
                    <Route element={<ProtectedRoute/>}>
                        <Route element={<AppShell/>}>
                            <Route path="/" element={<GalleryPage/>}/>
                            <Route path="/photos/:id" element={<PhotoPage/>}/>
                            <Route path="/tags" element={<TagsPage/>}/>
                            <Route path="/tagging" element={<TaggingPage/>}/>
                            <Route path="/tagging/:id" element={<ServiceEditorPage/>}/>
                            <Route path="/shares" element={<SharesPage/>}/>
                            <Route path="/settings" element={<SettingsPage/>}/>
                            <Route path="/trash" element={<TrashPage/>}/>
                        </Route>
                    </Route>

                    {/* Admin-only */}
                    <Route element={<ProtectedRoute adminOnly/>}>
                        <Route element={<AppShell/>}>
                            <Route path="/admin" element={<AdminPage/>}/>
                        </Route>
                    </Route>

                    <Route path="*" element={<Navigate to="/" replace/>}/>
                </Routes>
            </BrowserRouter>
            <Toaster richColors position="top-right"/>
        </QueryClientProvider>
    )
}
