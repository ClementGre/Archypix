import {Tabs, TabsContent, TabsList, TabsTrigger} from '@/components/ui/tabs'
import {OverviewTab} from '@/components/admin/OverviewTab'
import {UsersTab} from '@/components/admin/UsersTab'
import {JobsTab} from '@/components/admin/JobsTab'
import {SharesTab} from '@/components/admin/SharesTab'

export default function AdminPage() {
    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-5xl space-y-6">
                <h1 className="text-xl font-semibold">Admin</h1>
                <Tabs defaultValue="overview">
                    <TabsList>
                        <TabsTrigger value="overview">Overview</TabsTrigger>
                        <TabsTrigger value="users">Users</TabsTrigger>
                        <TabsTrigger value="jobs">Jobs</TabsTrigger>
                        <TabsTrigger value="shares">Shares & Federation</TabsTrigger>
                    </TabsList>
                    <TabsContent value="overview" className="mt-6">
                        <OverviewTab/>
                    </TabsContent>
                    <TabsContent value="users" className="mt-6">
                        <UsersTab/>
                    </TabsContent>
                    <TabsContent value="jobs" className="mt-6">
                        <JobsTab/>
                    </TabsContent>
                    <TabsContent value="shares" className="mt-6">
                        <SharesTab/>
                    </TabsContent>
                </Tabs>
            </div>
        </div>
    )
}
