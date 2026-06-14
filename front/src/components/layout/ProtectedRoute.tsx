import {Navigate, Outlet, useLocation} from 'react-router-dom'
import {useAuthStore} from '@/stores/auth'

/** Gates child routes behind authentication (and optionally admin role). */
export function ProtectedRoute({adminOnly = false}: { adminOnly?: boolean }) {
    const user = useAuthStore((s) => s.user)
    const location = useLocation()

    if (!user) {
        return <Navigate to="/login" replace state={{from: location}}/>
    }
    if (adminOnly && !user.is_admin) {
        return <Navigate to="/" replace/>
    }
    return <Outlet/>
}
