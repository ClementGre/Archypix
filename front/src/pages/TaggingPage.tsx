import {useMutation} from '@tanstack/react-query'
import {Play} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Skeleton} from '@/components/ui/skeleton'
import {NewServiceMenu} from '@/components/tagging/NewServiceMenu'
import {SharedMappingSection} from '@/components/tagging/SharedMappingSection'
import {PipelineList} from '@/components/tagging/PipelineList'
import {useTaggingMutations, useTaggingServices} from '@/hooks/useTaggingServices'
import {wakePipeline} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import type {RuleServiceDetail, SegmentationServiceDetail, ServiceType, SharedTagMappingServiceDetail} from '@/lib/types'

export default function TaggingPage() {
    const {data: services, isLoading, error} = useTaggingServices()
    const {create} = useTaggingMutations()

    const forceRun = useMutation({
        mutationFn: wakePipeline,
        onSuccess: () => toast.success('Pipeline run triggered'),
        onError: (err) => toast.error(apiErrorMessage(err)),
    })

    const handleCreate = (service_type: ServiceType) => {
        create.mutate(
            {service_type},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    const mappingServices: SharedTagMappingServiceDetail[] = (services ?? []).filter(
        (s): s is SharedTagMappingServiceDetail => s.service_type === 'shared_tag_mapping',
    )

    const pipelineServices: (RuleServiceDetail | SegmentationServiceDetail)[] = (
        services ?? []
    )
        .filter(
            (s): s is RuleServiceDetail | SegmentationServiceDetail =>
                s.service_type === 'rule' || s.service_type === 'segmentation',
        )
        .sort((a, b) => a.position - b.position)

    return (
        <div className="h-full overflow-y-auto p-6">
            <div className="mx-auto max-w-4xl">
                {/* Header */}
                <div className="flex items-center justify-between mb-6">
                    <div>
                        <h1 className="text-xl font-semibold">Tagging pipeline</h1>
                        <p className="mt-0.5 text-sm text-muted-foreground">
                            Ordered pipeline of services that auto-assign tags to pictures. Drag to reorder.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <Button
                            variant="outline"
                            size="sm"
                            onClick={() => forceRun.mutate()}
                            disabled={forceRun.isPending}
                            title="Re-run the tagging pipeline now (useful for debugging)"
                        >
                            <Play className="mr-1.5 h-3.5 w-3.5"/>
                            Force run
                        </Button>
                        <NewServiceMenu onCreate={handleCreate} isPending={create.isPending}/>
                    </div>
                </div>

                {/* Loading */}
                {isLoading && (
                    <div className="space-y-3">
                        <Skeleton className="h-16 w-full"/>
                        <Skeleton className="h-24 w-full"/>
                        <Skeleton className="h-24 w-full"/>
                    </div>
                )}

                {/* Error */}
                {error && !isLoading && (
                    <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
                        {apiErrorMessage(error)}
                    </div>
                )}

                {/* Content */}
                {!isLoading && !error && services !== undefined && (
                    <>
                        {/* Shared-tag mapping services — collapsed by default, always first */}
                        <SharedMappingSection services={mappingServices}/>

                        {/* Pipeline (rule + segmentation), drag-reorderable */}
                        <div>
                            <h2 className="text-sm font-medium text-muted-foreground mb-3 uppercase tracking-wide">
                                Pipeline
                            </h2>
                            <PipelineList services={pipelineServices}/>
                        </div>
                    </>
                )}
            </div>
        </div>
    )
}
