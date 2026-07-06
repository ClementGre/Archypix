import {Skeleton} from '@/components/ui/skeleton'
import {SettingsPanel} from '@/components/admin/SettingsPanel'
import {useAdminSettings, useAdminSettingMutations} from '@/hooks/useAdmin'

/** This instance's runtime configuration (feature 23 §4.5) via the shared metadata-driven panel. */
export function SettingsTab() {
    const {data, isLoading} = useAdminSettings()
    const {patch, reset} = useAdminSettingMutations()

    if (isLoading || !data) {
        return <div className="space-y-4">{Array.from({length: 5}).map((_, i) => <Skeleton key={i} className="h-24 w-full"/>)}</div>
    }

    return (
        <div className="space-y-4">
            <p className="text-sm text-muted-foreground">
                Operational settings for this instance. Fields set by an environment variable are locked and
                shown read-only; changes take effect live unless marked restart-required.
            </p>
            <SettingsPanel
                fields={data}
                onPatch={(key, value) => patch.mutateAsync({key, value}).then(() => undefined)}
                onReset={(key) => reset.mutateAsync(key).then(() => undefined)}
            />
        </div>
    )
}
