import {useCallback, useState} from 'react'
import {ArrowLeft, Loader2, Save, Undo2} from 'lucide-react'
import {toast} from 'sonner'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Separator} from '@/components/ui/separator'
import {TagListField} from '@/components/tags/TagListField'
import {ServiceActions} from './ServiceActions'
import {PruneSettings} from './PruneSettings'
import {RuleEditor} from './RuleEditor'
import {SegmentEditor} from './SegmentEditor'
import {MappingEditor} from './MappingEditor'
import {SERVICE_TYPE_COLOR, SERVICE_TYPE_LABEL} from './serviceTypes'
import type {ConfigDraft} from './useReportDraft'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {useRegisterUnsaved} from '@/stores/unsavedChanges'
import {apiErrorMessage} from '@/api/client'
import type {ServiceDetailResponse, ServicePatch} from '@/lib/types'

/** The service-level fields edited as one draft beside the type-specific config. */
type Settings = Required<Omit<ServicePatch, 'enabled' | 'promote_tags'>>

const settingsOf = (s: ServiceDetailResponse): Settings => ({
    name: s.name,
    requires: s.requires,
    excludes: s.excludes,
    prune_shadowed_manual_tags: s.prune_shadowed_manual_tags,
    keep_manual_tags: s.keep_manual_tags,
})

interface ServiceEditorProps {
    service: ServiceDetailResponse
    onBack: () => void
    onDeleted: () => void
}

/**
 * Right-pane editor for the selected service. Every field is one draft with one Save / Reset in a
 * fixed header: service-level fields go through `PATCH`, the type config through `PUT …/config`.
 */
export function ServiceEditor({service, onBack, onDeleted}: ServiceEditorProps) {
    const {update, replaceConfig} = useTaggingMutations()
    const server = settingsOf(service)
    const serverKey = JSON.stringify(server)
    const [settings, setSettings] = useState(server)
    const [syncedKey, setSyncedKey] = useState(serverKey)
    if (serverKey !== syncedKey) {
        setSettings(server)
        setSyncedKey(serverKey)
    }
    const [configDraft, setConfigDraft] = useState<ConfigDraft | null>(null)
    const onDraft = useCallback((d: ConfigDraft) => setConfigDraft(d), [])
    // Remounting the type editor resets its draft.
    const [resetKey, setResetKey] = useState(0)

    const set = (patch: Partial<Settings>) => setSettings((s) => ({...s, ...patch}))
    const settingsDirty = JSON.stringify(settings) !== serverKey
    const dirty = settingsDirty || !!configDraft?.dirty
    const pending = update.isPending || replaceConfig.isPending

    const save = async (): Promise<boolean> => {
        try {
            if (settingsDirty) {
                const changed = Object.fromEntries(
                    Object.entries(settings).filter(([k, v]) => JSON.stringify(v) !== JSON.stringify(server[k as keyof Settings])),
                ) as ServicePatch
                if (changed.name !== undefined) changed.name = changed.name.trim()
                await update.mutateAsync({id: service.id, body: changed})
            }
            if (configDraft?.dirty) await replaceConfig.mutateAsync({id: service.id, config: configDraft.config})
            toast.success('Service saved')
            return true
        } catch (err) {
            toast.error(apiErrorMessage(err))
            return false
        }
    }
    const reset = () => {
        setSettings(server)
        setResetKey((k) => k + 1)
    }
    useRegisterUnsaved(`service:${service.id}`, dirty, save)

    const typeLabel = SERVICE_TYPE_LABEL[service.service_type]

    return (
        <div className="flex flex-col md:h-full">
            {/* md+: outside the scrolling body. Below md the document scrolls, so it pins under the top bar. */}
            <div className="sticky top-topbar z-[5] border-b bg-background/95 backdrop-blur md:static md:shrink-0">
                <div className="mx-auto flex max-w-3xl flex-wrap items-center gap-2 px-5 py-3">
                    <Button variant="ghost" size="icon" onClick={onBack} className="-ml-2 h-8 w-8 text-muted-foreground lg:hidden"
                            aria-label="All services">
                        <ArrowLeft className="h-4 w-4"/>
                    </Button>
                    <Badge variant="secondary" className={`border-0 font-medium ${SERVICE_TYPE_COLOR[service.service_type]}`}>
                        {typeLabel}
                    </Badge>
                    <Input
                        value={settings.name}
                        onChange={(e) => set({name: e.target.value})}
                        placeholder={`Unnamed ${typeLabel.toLowerCase()}`}
                        maxLength={255}
                        className="h-8 min-w-0 flex-1 basis-40 font-medium"
                        aria-label="Service name"
                    />
                    <span className="text-sm text-muted-foreground">{service.enabled ? 'Enabled' : 'Disabled'}</span>
                    {dirty && (
                        <Button variant="ghost" size="sm" className="h-8 gap-1.5" onClick={reset} disabled={pending}>
                            <Undo2 className="h-4 w-4"/>
                            Reset
                        </Button>
                    )}
                    <Button size="sm" className="h-8 gap-1.5" onClick={() => void save()} disabled={!dirty || pending}>
                        {pending ? <Loader2 className="h-4 w-4 animate-spin"/> : <Save className="h-4 w-4"/>}
                        Save
                    </Button>
                    <ServiceActions service={service} onDeleted={onDeleted}/>
                </div>
            </div>

            <div className="md:min-h-0 md:flex-1 md:overflow-y-auto">
                <div className="mx-auto max-w-3xl space-y-5 p-5">
                    {/* Gates */}
                    <section>
                        <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">Gates</h3>
                        <div className="space-y-2 rounded-lg border p-3 text-sm">
                            <TagListField
                                label="Requires"
                                values={settings.requires}
                                onChange={(requires) => set({requires})}
                                excludePaths={settings.excludes}
                                color="emerald"
                                allowCreate={false}
                                emptyHint="None"
                            />
                            <TagListField
                                label="Excludes"
                                values={settings.excludes}
                                onChange={(excludes) => set({excludes})}
                                excludePaths={settings.requires}
                                color="red"
                                allowCreate={false}
                                emptyHint="None"
                            />
                        </div>
                    </section>

                    {/* Manual tag pruning */}
                    <section>
                        <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">Manual tags</h3>
                        <div className="rounded-lg border p-3">
                            <PruneSettings
                                prune={settings.prune_shadowed_manual_tags}
                                keep={settings.keep_manual_tags}
                                onChange={set}
                            />
                        </div>
                    </section>

                    <Separator/>

                    {/* Type-specific config */}
                    <section key={resetKey}>
                        {service.service_type === 'rule' && <RuleEditor rules={service.rules} onDraft={onDraft}/>}
                        {service.service_type === 'segmentation' && <SegmentEditor service={service} onDraft={onDraft}/>}
                        {service.service_type === 'shared_tag_mapping' && <MappingEditor service={service} onDraft={onDraft}/>}
                    </section>
                </div>
            </div>
        </div>
    )
}
