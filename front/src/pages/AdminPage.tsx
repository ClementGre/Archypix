import {useNavigate} from 'react-router-dom'
import {useQueryClient} from '@tanstack/react-query'
import {Network, RefreshCw} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {AdminDashboard} from '@/components/admin/AdminDashboard'

export default function AdminPage() {
    const queryClient = useQueryClient()
    const navigate = useNavigate()

    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-5xl space-y-6">
                <div className="flex items-center justify-between gap-3">
                    <h1 className="text-xl font-semibold">Admin</h1>
                    <div className="flex items-center gap-2">
                        <Button
                            variant="outline"
                            size="sm"
                            className="gap-1.5"
                            onClick={() => navigate('/admin/resolver')}
                            title="Fleet dashboard (resolver operator)"
                        >
                            <Network className="h-4 w-4"/>
                            Fleet
                        </Button>
                        <Button
                            variant="ghost"
                            size="icon"
                            title="Refresh (clear cache)"
                            aria-label="Refresh"
                            onClick={() => queryClient.invalidateQueries({queryKey: ['admin']})}
                        >
                            <RefreshCw className="h-4 w-4"/>
                        </Button>
                    </div>
                </div>
                {/* No provider needed — useAdminClient defaults to the user's apiClient (scope 'local'). */}
                <AdminDashboard/>
            </div>
        </div>
    )
}
