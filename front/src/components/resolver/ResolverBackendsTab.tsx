import {useState} from 'react'
import {ArrowLeft, ArrowRightCircle, ChevronRight, HardDrive, Image, Server, Users} from 'lucide-react'
import {Card, CardContent} from '@/components/ui/card'
import {Button} from '@/components/ui/button'
import {Skeleton} from '@/components/ui/skeleton'
import {BackendHealthRow} from '@/components/resolver/BackendHealthList'
import {BackendDrillDown} from '@/components/resolver/BackendDrillDown'
import {useNextBackend, useResolverOverview} from '@/hooks/useResolverAdmin'
import {formatBytes} from '@/lib/utils'

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
                    <div className="rounded-md bg-muted p-2"><Icon className="h-4 w-4 text-muted-foreground"/></div>
                    <div className="min-w-0">
                        <p className="text-2xl font-semibold tabular-nums">{value}</p>
                        <p className="text-xs text-muted-foreground">{label}</p>
                        {sub && <p className="text-xs text-muted-foreground">{sub}</p>}
                    </div>
                </div>
            </CardContent>
        </Card>
    )
}

/**
 * Fleet overview **and** backends (merged, feature 24) — fleet Σ stat cards + the clickable backend
 * list (drill into a backend's full proxied dashboard) + where the next signup lands.
 */
export function ResolverBackendsTab({refetchInterval}: { refetchInterval: number | false }) {
    const {data, isLoading} = useResolverOverview({refetchInterval})
    const {data: nextBackend} = useNextBackend({refetchInterval})
    const [selected, setSelected] = useState<string | null>(null)

    if (isLoading || !data) {
        return (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                {Array.from({length: 4}).map((_, i) => (
                    <Card key={i}><CardContent className="pt-6"><Skeleton className="h-10 w-full"/></CardContent></Card>
                ))}
            </div>
        )
    }

    const active = selected ? data.backends.find((b) => b.back_domain === selected) : null

    // Detail view: a backend's full admin dashboard (its own tab bar); capacity + fleet state live in
    // that dashboard's "Resolver" tab.
    if (active) {
        return (
            <div className="space-y-4">
                <div className="flex items-center gap-2">
                    <Button variant="ghost" size="sm" className="gap-1.5" onClick={() => setSelected(null)}>
                        <ArrowLeft className="h-4 w-4"/> All backends
                    </Button>
                    <span className="font-mono text-sm font-medium">{active.back_domain}</span>
                </div>
                <BackendDrillDown b={active}/>
            </div>
        )
    }

    return (
        <div className="space-y-6">
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                <StatCard icon={Users} label="Users (fleet)" value={data.total_users.toLocaleString()}/>
                <StatCard icon={Image} label="Pictures (fleet)" value={data.total_pictures.toLocaleString()}/>
                <StatCard icon={HardDrive} label="Storage (fleet)" value={formatBytes(data.total_storage_bytes)}/>
                <StatCard icon={Server} label="Backends" value={data.backend_count} sub={`${data.reachable_count} reachable`}/>
            </div>

            <div className="space-y-2">
                {data.backends.length === 0 ? (
                    <p className="text-sm text-muted-foreground">No backends have registered with this resolver yet.</p>
                ) : (
                    <>
                        <Card className="overflow-hidden py-0">
                            {data.backends.map((b) => (
                                <button
                                    key={b.back_domain}
                                    className="flex w-full items-center gap-2 px-4 text-left transition-colors hover:bg-muted/50"
                                    onClick={() => setSelected(b.back_domain)}
                                >
                                    <div className="min-w-0 flex-1"><BackendHealthRow b={b}/></div>
                                    <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground"/>
                                </button>
                            ))}
                        </Card>
                        <p className="flex items-center gap-1.5 px-1 text-xs text-muted-foreground">
                            <ArrowRightCircle className="h-3.5 w-3.5"/>
                            Next signup →{' '}
                            {nextBackend
                                ? <span className="font-mono font-medium text-foreground">{nextBackend}</span>
                                : <span className="text-amber-600 dark:text-amber-500">no eligible backend (all full / closed / unreachable)</span>}
                        </p>
                    </>
                )}
            </div>
        </div>
    )
}
