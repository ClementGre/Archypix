import {AlertTriangle, Loader2, Play, Timer} from 'lucide-react'
import {toast} from 'sonner'
import {Card, CardContent, CardHeader, CardTitle} from '@/components/ui/card'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Skeleton} from '@/components/ui/skeleton'
import {SettingsPanel} from '@/components/admin/SettingsPanel'
import {useAdminRoutines, useAdminSettingMutations, useTriggerRoutine} from '@/hooks/useAdmin'
import {apiErrorMessage} from '@/api/client'
import type {RoutineInfo} from '@/lib/types'

function relTime(unixSecs: number | null): string {
    if (!unixSecs) return 'never'
    const diff = Date.now() - unixSecs * 1000
    const s = Math.floor(diff / 1000)
    if (s < 60) return `${s}s ago`
    const m = Math.floor(s / 60)
    if (m < 60) return `${m}m ago`
    const h = Math.floor(m / 60)
    if (h < 24) return `${h}h ago`
    return `${Math.floor(h / 24)}d ago`
}

export interface RoutineHandlers {
    onTrigger: (name: string) => Promise<void>
    onPatch: (key: string, value: unknown) => Promise<void>
    onReset: (key: string) => Promise<void>
    triggering: boolean
}

function RoutineCard({routine, handlers}: { routine: RoutineInfo; handlers: RoutineHandlers }) {
    const running = routine.in_flight > 0

    const trigger = async () => {
        try {
            await handlers.onTrigger(routine.name)
            toast.success(`Triggered ${routine.name}`)
        } catch (e) {
            toast.error('Could not trigger routine', {description: apiErrorMessage(e)})
        }
    }

    return (
        <Card>
            <CardHeader className="pb-3">
                <div className="flex items-center justify-between gap-3">
                    <CardTitle className="flex items-center gap-2 text-sm font-medium">
                        <Timer className="h-4 w-4"/>
                        <span className="font-mono">{routine.name}</span>
                        {running
                            ? <Badge variant="secondary" className="gap-1 bg-primary/15 text-primary"><Loader2
                                className="h-3 w-3 animate-spin"/> running ×{routine.in_flight}</Badge>
                            : <Badge variant="secondary" className="text-muted-foreground">idle</Badge>}
                    </CardTitle>
                    <Button size="sm" variant="outline" className="h-7 gap-1.5" onClick={trigger} disabled={handlers.triggering}>
                        <Play className="h-3.5 w-3.5"/> Trigger now
                    </Button>
                </div>
            </CardHeader>
            <CardContent className="space-y-3">
                <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-xs sm:grid-cols-4">
                    <div>
                        <dt className="text-muted-foreground">Last started</dt>
                        <dd>{relTime(routine.last_started_at)}</dd>
                    </div>
                    <div>
                        <dt className="text-muted-foreground">Last finished</dt>
                        <dd>{relTime(routine.last_finished_at)}</dd>
                    </div>
                    <div>
                        <dt className="text-muted-foreground">Total runs</dt>
                        <dd className="tabular-nums">{routine.total_runs.toLocaleString()}</dd>
                    </div>
                    <div>
                        <dt className="text-muted-foreground">In flight</dt>
                        <dd className="tabular-nums">{routine.in_flight}</dd>
                    </div>
                </dl>
                {routine.last_error && (
                    <p className="flex items-start gap-1.5 rounded-md bg-red-500/10 px-2 py-1.5 text-xs text-red-500">
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0"/>
                        <span className="break-all">{routine.last_error}</span>
                    </p>
                )}
                {routine.settings.length > 0 && (
                    <div className="border-t border-border/60 pt-2">
                        <SettingsPanel flat fields={routine.settings} onPatch={handlers.onPatch} onReset={handlers.onReset}/>
                    </div>
                )}
            </CardContent>
        </Card>
    )
}

/** Presentational routines list — driven by whatever data + handlers the caller provides. */
export function RoutinesView({routines, isLoading, handlers}: {
    routines: RoutineInfo[] | undefined
    isLoading: boolean
    handlers: RoutineHandlers
}) {
    if (isLoading) {
        return <div className="space-y-3">{Array.from({length: 3}).map((_, i) => <Skeleton key={i} className="h-32 w-full"/>)}</div>
    }
    if (!routines || routines.length === 0) {
        return <p className="text-sm text-muted-foreground">No routines are running.</p>
    }
    return <div className="space-y-3">{routines.map((r) => <RoutineCard key={r.name} routine={r} handlers={handlers}/>)}</div>
}

/** Backend `/admin` Routines tab. */
export function RoutinesPanel() {
    const {data, isLoading} = useAdminRoutines({refetchInterval: 10_000})
    const trigger = useTriggerRoutine()
    const {patch, reset} = useAdminSettingMutations()
    return (
        <RoutinesView
            routines={data}
            isLoading={isLoading}
            handlers={{
                triggering: trigger.isPending,
                onTrigger: (name) => trigger.mutateAsync(name).then(() => undefined),
                onPatch: (key, value) => patch.mutateAsync({key, value}).then(() => undefined),
                onReset: (key) => reset.mutateAsync(key).then(() => undefined),
            }}
        />
    )
}
