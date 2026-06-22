import {toast} from 'sonner'
import {Accordion, AccordionContent, AccordionItem, AccordionTrigger,} from '@/components/ui/accordion'
import {Badge} from '@/components/ui/badge'
import {Card, CardContent} from '@/components/ui/card'
import {Separator} from '@/components/ui/separator'
import {MappingEditor} from './MappingEditor'
import {DeleteServiceDialog} from './DeleteServiceDialog'
import {ServiceNameEditor} from './ServiceNameEditor'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {apiErrorMessage} from '@/api/client'
import type {SharedTagMappingServiceDetail} from '@/lib/types'

interface SharedMappingSectionProps {
    services: SharedTagMappingServiceDetail[]
}

export function SharedMappingSection({services}: SharedMappingSectionProps) {
    const {remove, update} = useTaggingMutations()

    if (services.length === 0) return null

    const handleDelete = (id: string, promoteTags: boolean) => {
        remove.mutate(
            {id, promoteTags},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    return (
        <div className="mb-6">
            <Accordion type="single" collapsible defaultValue="">
                <AccordionItem value="shared-mappings" className="border rounded-lg px-4">
                    <AccordionTrigger className="hover:no-underline">
                        <div className="flex items-center gap-2">
                            <span className="font-medium text-sm">Shared-tag mappings</span>
                            <Badge
                                variant="secondary"
                                className="border-0 bg-amber-500/15 text-amber-500 text-xs"
                            >
                                {services.length} service{services.length !== 1 ? 's' : ''}
                            </Badge>
                            <span className="text-xs text-muted-foreground ml-1">
                                — run first, not reorderable
                            </span>
                        </div>
                    </AccordionTrigger>
                    <AccordionContent>
                        <div className="space-y-3 pb-2">
                            {services.map((service) => (
                                <Card key={service.id} className="border">
                                    <CardContent className="p-4">
                                        <div className="flex items-center justify-between mb-3">
                                            <div className="flex items-center gap-2">
                                                <Badge
                                                    variant="secondary"
                                                    className="border-0 bg-amber-500/15 text-amber-500"
                                                >
                                                    Shared-tag mapping
                                                </Badge>
                                                <ServiceNameEditor
                                                    name={service.name}
                                                    placeholder="Unnamed mapping"
                                                    onRename={(name) =>
                                                        update.mutate(
                                                            {id: service.id, body: {name}},
                                                            {onError: (err) => toast.error(apiErrorMessage(err))},
                                                        )
                                                    }
                                                    isPending={update.isPending}
                                                />
                                                <span className="text-xs text-muted-foreground">
                                                    {service.mappings.length} mapping
                                                    {service.mappings.length !== 1 ? 's' : ''}
                                                </span>
                                            </div>
                                            <DeleteServiceDialog
                                                onDelete={(promoteTags) =>
                                                    handleDelete(service.id, promoteTags)
                                                }
                                                isPending={remove.isPending}
                                            />
                                        </div>
                                        <Separator className="mb-3"/>
                                        <MappingEditor
                                            serviceId={service.id}
                                            mappings={service.mappings}
                                        />
                                    </CardContent>
                                </Card>
                            ))}
                        </div>
                    </AccordionContent>
                </AccordionItem>
            </Accordion>
        </div>
    )
}
