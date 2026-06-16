import {Activity, AlertTriangle, CheckCircle2, Clock, Database, Image, RefreshCw, Server, Users, XCircle} from 'lucide-react'
import {Card, CardContent, CardHeader, CardTitle} from '@/components/ui/card'
import {Badge} from '@/components/ui/badge'
import {Skeleton} from '@/components/ui/skeleton'
import {useConsistencyCheck, useInstanceHealth, useInstanceStats} from '@/hooks/useAdmin'

function formatBytes(bytes: number): string {
    if (bytes === 0) return '0 B'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    const i = Math.floor(Math.log(bytes) / Math.log(1024))
    return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`
}

function relativeTime(iso: string | null): string {
    if (!iso) return 'never'
    const diff = Date.now() - new Date(iso).getTime()
    const mins = Math.floor(diff / 60_000)
    if (mins < 1) return 'just now'
    if (mins < 60) return `${mins}m ago`
    const hrs = Math.floor(mins / 60)
    if (hrs < 24) return `${hrs}h ago`
    return `${Math.floor(hrs / 24)}d ago`
}

function StatusDot({ok}: { ok: boolean }) {
    return (
        <span
            className={`inline-block h-2 w-2 rounded-full ${ok ? 'bg-emerald-500' : 'bg-red-500'}`}
        />
    )
}

function HealthCard() {
    const {data, isLoading, error} = useInstanceHealth()

    return (
        <Card>
            <CardHeader className="pb-3">
                <CardTitle className="flex items-center gap-2 text-sm font-medium">
                    <Server className="h-4 w-4"/>
                    Instance health
                </CardTitle>
            </CardHeader>
            <CardContent>
                {isLoading ? (
                    <div className="space-y-2">
                        {Array.from({length: 4}).map((_, i) => <Skeleton key={i} className="h-5 w-full"/>)}
                    </div>
                ) : error ? (
                    <p className="text-sm text-destructive">Failed to load health data</p>
                ) : data ? (
                    <dl className="space-y-2 text-sm">
                        <div className="flex justify-between">
                            <dt className="text-muted-foreground">Global domain</dt>
                            <dd className="font-mono">{data.global_domain}</dd>
                        </div>
                        <div className="flex justify-between">
                            <dt className="text-muted-foreground">Backend domain</dt>
                            <dd className="font-mono">{data.back_domain}</dd>
                        </div>
                        <div className="flex justify-between items-center">
                            <dt className="text-muted-foreground">Database</dt>
                            <dd className="flex items-center gap-1.5">
                                <StatusDot ok={data.db_connected}/>
                                {data.db_connected ? 'Connected' : 'Disconnected'}
                            </dd>
                        </div>
                        <div className="flex justify-between items-center">
                            <dt className="text-muted-foreground">Redis</dt>
                            <dd className="flex items-center gap-1.5">
                                <StatusDot ok={data.redis_connected}/>
                                {data.redis_connected ? 'Connected' : 'Disconnected'}
                            </dd>
                        </div>
                        <div className="flex justify-between items-center">
                            <dt className="text-muted-foreground">Last worker activity</dt>
                            <dd className="flex items-center gap-1.5">
                                <Clock className="h-3.5 w-3.5 text-muted-foreground"/>
                                {relativeTime(data.last_worker_activity_at)}
                            </dd>
                        </div>
                    </dl>
                ) : null}
            </CardContent>
        </Card>
    )
}

function StatCard({icon: Icon, label, value, sub}: {
    icon: React.ElementType
    label: string
    value: string | number
    sub?: string
}) {
    return (
        <Card>
            <CardContent className="pt-6">
                <div className="flex items-center gap-3">
                    <div className="rounded-md bg-muted p-2">
                        <Icon className="h-4 w-4 text-muted-foreground"/>
                    </div>
                    <div>
                        <p className="text-2xl font-semibold tabular-nums">{value}</p>
                        <p className="text-xs text-muted-foreground">{label}</p>
                        {sub && <p className="text-xs text-muted-foreground">{sub}</p>}
                    </div>
                </div>
            </CardContent>
        </Card>
    )
}

function StatsGrid() {
    const {data, isLoading} = useInstanceStats()

    if (isLoading) {
        return (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
                {Array.from({length: 6}).map((_, i) => (
                    <Card key={i}>
                        <CardContent className="pt-6">
                            <Skeleton className="h-10 w-full"/>
                        </CardContent>
                    </Card>
                ))}
            </div>
        )
    }

    if (!data) return null

    const total = data.job_counts.pending + data.job_counts.processing + data.job_counts.completed + data.job_counts.failed

    return (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
            <StatCard icon={Users} label="Users" value={data.user_count}/>
            <StatCard
                icon={Image}
                label="Pictures"
                value={(data.owned_picture_count + data.received_picture_count).toLocaleString()}
                sub={`${data.owned_picture_count.toLocaleString()} owned · ${data.received_picture_count.toLocaleString()} received`}
            />
            <StatCard icon={Database} label="Storage" value={formatBytes(data.total_storage_bytes)}/>
            <StatCard
                icon={Activity}
                label="Jobs (all time)"
                value={total.toLocaleString()}
                sub={`${data.job_counts.pending} pending · ${data.job_counts.processing} running`}
            />
            <StatCard
                icon={RefreshCw}
                label="Dirty pictures"
                value={data.dirty_picture_count.toLocaleString()}
                sub="awaiting pipeline"
            />
            <StatCard
                icon={AlertTriangle}
                label="Errored shares"
                value={data.errored_share_count}
            />
        </div>
    )
}

function ConsistencyCard() {
    const {data, isLoading} = useConsistencyCheck()

    const items = data
        ? [
            {
                label: 'Stuck EXIF sync jobs',
                value: data.stuck_exif_pending_count,
                desc: 'Pictures stuck in pending without an active job',
            },
            {
                label: 'Pictures without thumbnail',
                value: data.pictures_without_thumbnail_count,
                desc: 'Older than 30 min with no thumbnail generated',
            },
            {
                label: 'Broken share mappings',
                value: data.broken_mapping_count,
                desc: 'SharedTagMapping entries referencing a revoked share',
            },
        ]
        : []

    const allClear = items.every((i) => i.value === 0)

    return (
        <Card>
            <CardHeader className="pb-3">
                <CardTitle className="flex items-center gap-2 text-sm font-medium">
                    {allClear ? (
                        <CheckCircle2 className="h-4 w-4 text-emerald-500"/>
                    ) : (
                        <XCircle className="h-4 w-4 text-red-500"/>
                    )}
                    Consistency check
                </CardTitle>
            </CardHeader>
            <CardContent>
                {isLoading ? (
                    <div className="space-y-2">
                        {Array.from({length: 3}).map((_, i) => <Skeleton key={i} className="h-5 w-full"/>)}
                    </div>
                ) : (
                    <ul className="space-y-2">
                        {items.map((item) => (
                            <li key={item.label} className="flex items-start justify-between gap-4 text-sm">
                                <div>
                                    <p>{item.label}</p>
                                    <p className="text-xs text-muted-foreground">{item.desc}</p>
                                </div>
                                <Badge
                                    variant="secondary"
                                    className={item.value === 0
                                        ? 'bg-emerald-500/15 text-emerald-500'
                                        : 'bg-red-500/15 text-red-500'
                                    }
                                >
                                    {item.value}
                                </Badge>
                            </li>
                        ))}
                    </ul>
                )}
            </CardContent>
        </Card>
    )
}

export function OverviewTab() {
    return (
        <div className="space-y-6">
            <StatsGrid/>
            <div className="grid gap-4 lg:grid-cols-2">
                <HealthCard/>
                <ConsistencyCard/>
            </div>
        </div>
    )
}
