import {ArrowLeft} from 'lucide-react'
import {toast} from 'sonner'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Switch} from '@/components/ui/switch'
import {Separator} from '@/components/ui/separator'
import {RequiresExcludesEditor} from './RequiresExcludesEditor'
import {ServiceNameEditor} from './ServiceNameEditor'
import {DeleteServiceDialog} from './DeleteServiceDialog'
import {RuleEditor} from './RuleEditor'
import {SegmentEditor} from './SegmentEditor'
import {MappingEditor} from './MappingEditor'
import {serviceTypeLabel} from './ServiceRow'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {ServiceDetailResponse, ServiceType} from '@/lib/types'

const TYPE_COLOR: Record<ServiceType, string> = {
    rule: 'bg-violet-500/15 text-violet-500',
    segmentation: 'bg-sky-500/15 text-sky-500',
    shared_tag_mapping: 'bg-amber-500/15 text-amber-500',
}

interface ServiceEditorProps {
    service: ServiceDetailResponse
    onBack: () => void
    onDeleted: () => void
}

/** Right-pane editor for the selected service: header + gates + type-specific config. */
export function ServiceEditor({service, onBack, onDeleted}: ServiceEditorProps) {
    const {update, remove} = useTaggingMutations()

    const patch = (body: { name?: string; enabled?: boolean; requires?: string[]; excludes?: string[] }) =>
        update.mutate({id: service.id, body}, {onError: (err) => toast.error(apiErrorMessage(err))})

    return (
        <div className="space-y-5">
            {/* Header */}
            <div>
                <Button variant="ghost" size="sm" onClick={onBack} className="mb-2 -ml-2 gap-1.5 text-muted-foreground lg:hidden">
                    <ArrowLeft className="h-4 w-4"/>
                    All services
                </Button>
                <div className="flex flex-wrap items-center gap-3">
                    <Badge variant="secondary" className={`border-0 font-medium ${TYPE_COLOR[service.service_type]}`}>
                        {serviceTypeLabel(service.service_type)}
                    </Badge>
                    <ServiceNameEditor
                        name={service.name}
                        placeholder={`Unnamed ${serviceTypeLabel(service.service_type).toLowerCase()}`}
                        onRename={(name) => patch({name})}
                        isPending={update.isPending}
                        className="text-base"
                    />
                    <div className="flex-1"/>
                    <label className="flex items-center gap-2 text-sm text-muted-foreground">
                        {service.enabled ? 'Enabled' : 'Disabled'}
                        <Switch checked={service.enabled} onCheckedChange={(enabled) => patch({enabled})} disabled={update.isPending}/>
                    </label>
                    <DeleteServiceDialog
                        isPending={remove.isPending}
                        onDelete={(promoteTags) =>
                            remove.mutate({id: service.id, promoteTags}, {onSuccess: onDeleted, onError: (err) => toast.error(apiErrorMessage(err))})
                        }
                    />
                </div>
            </div>

            {/* Gates */}
            <section>
                <h3 className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">Gates</h3>
                <div className="rounded-lg border p-3">
                    <RequiresExcludesEditor
                        requires={service.requires}
                        excludes={service.excludes}
                        onUpdate={patch}
                        isPending={update.isPending}
                    />
                </div>
            </section>

            <Separator/>

            {/* Type-specific config */}
            <section>
                {service.service_type === 'rule' && <RuleEditor serviceId={service.id} rules={service.rules}/>}
                {service.service_type === 'segmentation' && <SegmentEditor service={service}/>}
                {service.service_type === 'shared_tag_mapping' && <MappingEditor service={service}/>}
            </section>
        </div>
    )
}
