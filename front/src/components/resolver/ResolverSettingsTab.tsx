import {Skeleton} from '@/components/ui/skeleton'
import {SettingsPanel} from '@/components/admin/SettingsPanel'
import {useResolverSettings, useResolverSettingMutations} from '@/hooks/useResolverAdmin'

/** The resolver's own runtime config (selection strategy, registration mode, CORS, routine intervals…). */
export function ResolverSettingsTab() {
    const {data, isLoading} = useResolverSettings()
    const {patch, reset} = useResolverSettingMutations()

    if (isLoading || !data) {
        return <div className="space-y-4">{Array.from({length: 4}).map((_, i) => <Skeleton key={i} className="h-24 w-full"/>)}</div>
    }

    return (
        <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
                The resolver's own configuration. Fields set by an environment variable are locked and read-only.
            </p>
            <SettingsPanel
                fields={data}
                onPatch={(key, value) => patch.mutateAsync({key, value}).then(() => undefined)}
                onReset={(key) => reset.mutateAsync(key).then(() => undefined)}
            />
        </div>
    )
}
