import {Link, useParams} from 'react-router-dom'
import {ArrowLeft} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Switch} from '@/components/ui/switch'
import {Separator} from '@/components/ui/separator'
import {Skeleton} from '@/components/ui/skeleton'
import {RequiresExcludesEditor} from '@/components/tagging/RequiresExcludesEditor'
import {ServiceNameEditor} from '@/components/tagging/ServiceNameEditor'
import {RuleEditor} from '@/components/tagging/RuleEditor'
import {SegmentEditor} from '@/components/tagging/SegmentEditor'
import {MappingEditor} from '@/components/tagging/MappingEditor'
import {useTaggingMutations, useTaggingService} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'

const TYPE_LABEL: Record<string, string> = {
    rule: 'Rule',
    segmentation: 'Segmentation',
    shared_tag_mapping: 'Shared-tag mapping',
}

const TYPE_COLOR: Record<string, string> = {
    rule: 'bg-violet-500/15 text-violet-500',
    segmentation: 'bg-sky-500/15 text-sky-500',
    shared_tag_mapping: 'bg-amber-500/15 text-amber-500',
}

export default function ServiceEditorPage() {
    const {id} = useParams<{ id: string }>()
    const {data: service, isLoading, error} = useTaggingService(id ?? null)
    const {update} = useTaggingMutations()

    const handleToggle = (enabled: boolean) => {
        if (!service) return
        update.mutate(
            {id: service.id, body: {enabled}},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    const handleUpdateGates = (patch: { requires?: string[]; excludes?: string[] }) => {
        if (!service) return
        update.mutate(
            {id: service.id, body: patch},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-4xl">
                {/* Back link */}
                <Button variant="ghost" size="sm" asChild className="mb-4 -ml-2 gap-1.5 text-muted-foreground">
                    <Link to="/tagging">
                        <ArrowLeft className="h-4 w-4"/>
                        Tagging pipeline
                    </Link>
                </Button>

                {/* Loading */}
                {isLoading && (
                    <div className="space-y-4">
                        <Skeleton className="h-8 w-48"/>
                        <Skeleton className="h-6 w-full"/>
                        <Skeleton className="h-40 w-full"/>
                    </div>
                )}

                {/* Error */}
                {error && !isLoading && (
                    <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
                        {apiErrorMessage(error)}
                    </div>
                )}

                {/* Content */}
                {service && (
                    <>
                        {/* Header */}
                        <div className="flex items-center gap-3 mb-1">
                            <Badge
                                variant="secondary"
                                className={`border-0 font-medium ${TYPE_COLOR[service.service_type]}`}
                            >
                                {TYPE_LABEL[service.service_type]}
                            </Badge>
                            <ServiceNameEditor
                                name={service.name}
                                placeholder={`Unnamed ${TYPE_LABEL[service.service_type].toLowerCase()}`}
                                onRename={(name) =>
                                    update.mutate({id: service.id, body: {name}}, {onError: (err) => toast.error(apiErrorMessage(err))})
                                }
                                isPending={update.isPending}
                                className="text-lg"
                            />
                            <div className="flex-1"/>
                            <div className="flex items-center gap-2">
                                <span className="text-sm text-muted-foreground">
                                    {service.enabled ? 'Enabled' : 'Disabled'}
                                </span>
                                <Switch
                                    checked={service.enabled}
                                    onCheckedChange={handleToggle}
                                    disabled={update.isPending}
                                />
                            </div>
                        </div>
                        <p className="text-xs text-muted-foreground mb-6">ID: {service.id}</p>

                        {/* Requires / Excludes gates */}
                        <section className="mb-6">
                            <h2 className="text-sm font-medium mb-3">Gates</h2>
                            <div className="rounded-lg border p-4">
                                <RequiresExcludesEditor
                                    requires={service.requires}
                                    excludes={service.excludes}
                                    onUpdate={handleUpdateGates}
                                    isPending={update.isPending}
                                />
                            </div>
                        </section>

                        <Separator className="my-6"/>

                        {/* Type-specific editor */}
                        <section>
                            <h2 className="text-sm font-medium mb-3">
                                {service.service_type === 'rule' && 'Rules'}
                                {service.service_type === 'segmentation' && 'Segments'}
                                {service.service_type === 'shared_tag_mapping' && 'Mappings'}
                            </h2>

                            {service.service_type === 'rule' && (
                                <RuleEditor serviceId={service.id} rules={service.rules}/>
                            )}

                            {service.service_type === 'segmentation' && (
                                <SegmentEditor
                                    serviceId={service.id}
                                    segments={service.segments}
                                />
                            )}

                            {service.service_type === 'shared_tag_mapping' && (
                                <MappingEditor
                                    serviceId={service.id}
                                    mappings={service.mappings}
                                />
                            )}
                        </section>
                    </>
                )}
            </div>
        </div>
    )
}
