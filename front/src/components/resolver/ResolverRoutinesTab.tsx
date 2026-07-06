import {RoutinesView} from '@/components/admin/RoutinesPanel'
import {useResolverRoutines, useResolverSettingMutations, useTriggerResolverRoutine} from '@/hooks/useResolverAdmin'

/** The resolver's own background routines (stale-backend prune, invite cleanup) — feature 24. */
export function ResolverRoutinesTab({refetchInterval}: { refetchInterval: number | false }) {
    const {data, isLoading} = useResolverRoutines({refetchInterval})
    const trigger = useTriggerResolverRoutine()
    const {patch, reset} = useResolverSettingMutations()
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
