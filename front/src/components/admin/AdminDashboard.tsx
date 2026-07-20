import type {ReactNode} from 'react'
import {Tabs, TabsContent, TabsList, TabsTrigger} from '@/components/ui/tabs'
import {OverviewTab} from '@/components/admin/OverviewTab'
import {UsersTab} from '@/components/admin/UsersTab'
import {JobsTab} from '@/components/admin/JobsTab'
import {SharesTab} from '@/components/admin/SharesTab'
import {SettingsTab} from '@/components/admin/SettingsTab'
import {RoutinesPanel} from '@/components/admin/RoutinesPanel'
import {RateLimitsTab} from '@/components/admin/RateLimitsTab'
import {InvitesTab} from '@/components/admin/InvitesTab'

const TAB_LIST = 'inline-flex h-auto flex-wrap items-center justify-start gap-1 rounded-md bg-muted p-1'

export interface ExtraTab {
    value: string
    label: string
    content: ReactNode
}

/**
 * The full single-instance admin surface, transport-agnostic (feature 24 §5): the enclosing
 * `AdminClientProvider` decides whether these tabs hit the user's own backend (`/admin`) or a proxied
 * fleet backend. Invites are user-auth (not proxied), so the fleet drill-down omits that tab and passes
 * `extraTabs` (e.g. a resolver-side capacity/heartbeat tab) via a callback so nothing resolver-specific
 * leaks into the shared dashboard.
 */
export function AdminDashboard({showInvites = true, extraTabs = [], extraTabsFirst = false, defaultTab}: {
    showInvites?: boolean
    extraTabs?: ExtraTab[]
    /** Render the injected tabs before the standard ones (used by the fleet drill-down). */
    extraTabsFirst?: boolean
    /** Which tab opens by default (defaults to the first shown). */
    defaultTab?: string
}) {
    const extraTriggers = extraTabs.map((t) => <TabsTrigger key={t.value} value={t.value}>{t.label}</TabsTrigger>)
    const initial = defaultTab ?? (extraTabsFirst && extraTabs[0] ? extraTabs[0].value : 'overview')

    return (
        <Tabs defaultValue={initial}>
            <TabsList className={TAB_LIST}>
                {extraTabsFirst && extraTriggers}
                <TabsTrigger value="overview">Overview</TabsTrigger>
                <TabsTrigger value="users">Users</TabsTrigger>
                <TabsTrigger value="jobs">Jobs</TabsTrigger>
                <TabsTrigger value="shares">Shares &amp; Federation</TabsTrigger>
                <TabsTrigger value="settings">Settings</TabsTrigger>
                <TabsTrigger value="routines">Routines</TabsTrigger>
                <TabsTrigger value="rate-limits">Rate limiting</TabsTrigger>
                {showInvites && <TabsTrigger value="invites">Invites</TabsTrigger>}
                {!extraTabsFirst && extraTriggers}
            </TabsList>
            <TabsContent value="overview" className="mt-6"><OverviewTab/></TabsContent>
            <TabsContent value="users" className="mt-6"><UsersTab/></TabsContent>
            <TabsContent value="jobs" className="mt-6"><JobsTab/></TabsContent>
            <TabsContent value="shares" className="mt-6"><SharesTab/></TabsContent>
            <TabsContent value="settings" className="mt-6"><SettingsTab/></TabsContent>
            <TabsContent value="routines" className="mt-6"><RoutinesPanel/></TabsContent>
            <TabsContent value="rate-limits" className="mt-6"><RateLimitsTab/></TabsContent>
            {showInvites && <TabsContent value="invites" className="mt-6"><InvitesTab/></TabsContent>}
            {extraTabs.map((t) => <TabsContent key={t.value} value={t.value} className="mt-6">{t.content}</TabsContent>)}
        </Tabs>
    )
}
