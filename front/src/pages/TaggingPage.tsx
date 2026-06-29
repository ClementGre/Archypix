import {useState} from 'react'
import {useNavigate, useParams} from 'react-router-dom'
import {useMutation} from '@tanstack/react-query'
import {Play} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Skeleton} from '@/components/ui/skeleton'
import {NewServiceMenu} from '@/components/tagging/NewServiceMenu'
import {NewMappingDialog} from '@/components/tagging/NewMappingDialog'
import {SharedMappingSection} from '@/components/tagging/SharedMappingSection'
import {PipelineList} from '@/components/tagging/PipelineList'
import {ServiceEditor} from '@/components/tagging/ServiceEditor'
import {Section} from '@/components/photos/detail/Section'
import {useTaggingMutations, useTaggingServices} from '@/hooks/useTaggingServices'
import {wakePipeline} from '@/api/pictures'
import {apiErrorMessage} from '@/api/client'
import type {RuleServiceDetail, SegmentationServiceDetail, ServiceType, SharedTagMappingServiceDetail} from '@/lib/types'

export default function TaggingPage() {
    const {id: selectedId} = useParams<{ id: string }>()
    const navigate = useNavigate()
    const {data: services, isLoading, error} = useTaggingServices()
    const {create} = useTaggingMutations()
    const [mappingDialogOpen, setMappingDialogOpen] = useState(false)

    const select = (id: string | null) => navigate(id ? `/tagging/${id}` : '/tagging')

    const forceRun = useMutation({
        mutationFn: wakePipeline,
        onSuccess: () => toast.success('Pipeline run triggered'),
        onError: (err) => toast.error(apiErrorMessage(err)),
    })

    const handleCreate = (service_type: ServiceType) => {
        if (service_type === 'shared_tag_mapping') {
            setMappingDialogOpen(true)
            return
        }
        create.mutate(
            {service_type},
            {onSuccess: (svc) => select(svc.id), onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    const all = services ?? []
    const mappingServices = all.filter((s): s is SharedTagMappingServiceDetail => s.service_type === 'shared_tag_mapping')
    const pipelineServices = all
        .filter((s): s is RuleServiceDetail | SegmentationServiceDetail => s.service_type === 'rule' || s.service_type === 'segmentation')
        .sort((a, b) => a.position - b.position)
    const selected = all.find((s) => s.id === selectedId) ?? null

    return (
        <div className="flex h-full">
            {/* Left pane — service list */}
            <div className={`${selected ? 'hidden lg:flex' : 'flex'} w-full shrink-0 flex-col overflow-y-auto border-r lg:w-96`}>
                <div className="flex items-center justify-between gap-2 border-b p-4">
                    <div>
                        <h1 className="text-base font-semibold">Tagging services</h1>
                        <p className="text-xs text-muted-foreground">Auto-tag your photos. Drag to reorder.</p>
                    </div>
                    <div className="flex items-center gap-1.5">
                        <Button variant="outline" size="icon" className="h-8 w-8" onClick={() => forceRun.mutate()} disabled={forceRun.isPending}
                                title="Re-run tagging now (debug)">
                            <Play className="h-3.5 w-3.5"/>
                        </Button>
                        <NewServiceMenu onCreate={handleCreate} isPending={create.isPending}/>
                    </div>
                </div>

                <div className="flex-1 space-y-5 p-3">
                    {isLoading && (
                        <div className="space-y-2">
                            <Skeleton className="h-12 w-full"/>
                            <Skeleton className="h-12 w-full"/>
                            <Skeleton className="h-12 w-full"/>
                        </div>
                    )}
                    {error && !isLoading && (
                        <div
                            className="rounded-lg border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive">{apiErrorMessage(error)}</div>
                    )}

                    {!isLoading && !error && (
                        <>
                            <Section
                                id="rules-segments"
                                title="Rules & segments"
                                count={pipelineServices.length}
                                defaultOpen={true}
                            >
                                <PipelineList services={pipelineServices} selectedId={selectedId ?? null} onSelect={select}/>
                            </Section>
                            {mappingServices.length > 0 && (
                                <Section
                                    id="tagging-shared-mappings"
                                    title="Shared-tag mappings · run first"
                                    count={mappingServices.length}
                                    defaultOpen={false}
                                >
                                    <SharedMappingSection services={mappingServices} selectedId={selectedId ?? null} onSelect={select}/>
                                </Section>
                            )}
                            {all.length === 0 && (
                                <p className="rounded-lg border border-dashed py-10 text-center text-sm text-muted-foreground">
                                    No services yet — create one with &ldquo;New service&rdquo;.
                                </p>
                            )}
                        </>
                    )}
                </div>
            </div>

            {/* Right pane — editor */}
            <div className={`${selected ? 'block' : 'hidden lg:block'} flex-1 overflow-y-auto`}>
                {selected ? (
                    <div className="mx-auto max-w-3xl p-5">
                        <ServiceEditor service={selected} onBack={() => select(null)} onDeleted={() => select(null)}/>
                    </div>
                ) : (
                    <div className="hidden h-full items-center justify-center text-sm text-muted-foreground lg:flex">Select a service to edit
                        it.</div>
                )}
            </div>

            <NewMappingDialog
                open={mappingDialogOpen}
                onOpenChange={setMappingDialogOpen}
                mappedShareIds={mappingServices.map((s) => s.incoming_share_id)}
                onCreated={select}
            />
        </div>
    )
}
