import {useState} from 'react'
import {Save, Undo2} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Checkbox} from '@/components/ui/checkbox'
import {TagListField} from '@/components/tags/TagListField'
import {useTaggingServices} from '@/hooks/useTaggingServices'
import {TagPath} from '@/lib/utils'
import type {ServiceDetailResponse, ServicePatch} from '@/lib/types'

/** The tags (or tag root) a service can produce. */
function outputRoots(s: ServiceDetailResponse): string[] {
    switch (s.service_type) {
        case 'rule':
            return s.rules.map((r) => r.assign_tag)
        case 'segmentation':
            return [s.config.root_tag]
        case 'shared_tag_mapping':
            return s.assign_tags
    }
}

const related = (a: string, b: string) => a === b || a.startsWith(`${b}.`) || b.startsWith(`${a}.`)

/**
 * Shadowed manual tag pruning (feature 39 §4): the opt-in, the keep-list floor (a local draft,
 * saved once — every save re-runs the pipeline), and the gate arms that already anchor read-only.
 */
export function PruneSettings({
                                  service,
                                  onUpdate,
                                  isPending,
                              }: {
    service: ServiceDetailResponse
    onUpdate: (patch: ServicePatch) => void
    isPending: boolean
}) {
    const {data: services} = useTaggingServices()
    const [draft, setDraft] = useState(service.keep_manual_tags)
    const serverKey = service.keep_manual_tags.join('|')
    const [syncedKey, setSyncedKey] = useState(serverKey)
    if (serverKey !== syncedKey) {
        setDraft(service.keep_manual_tags)
        setSyncedKey(serverKey)
    }
    const dirty = draft.join('|') !== serverKey

    // Gate arms anchor what they cover (§4.4); shown so the keep-list need not repeat them (§4.5).
    const roots = outputRoots(service)
    const anchors = [...new Set((services ?? []).filter((s) => s.enabled).flatMap((s) => s.requires))]
        .filter((arm) => roots.some((root) => related(arm, root)))
        .sort()

    return (
        <div className="space-y-3 text-sm">
            <label className="flex items-start gap-2">
                <Checkbox
                    checked={service.prune_shadowed_manual_tags}
                    onCheckedChange={(v) => onUpdate({prune_shadowed_manual_tags: v === true})}
                    disabled={isPending}
                    className="mt-0.5"
                />
                <span>
                    Remove manual tags this service also produces — except at or above:
                    <span className="block text-xs text-muted-foreground">
                        The service then owns the tag; a manual copy (e.g. from a WebDAV move) is reduced to the
                        least it takes to keep the service's gates firing.
                    </span>
                </span>
            </label>
            <div className={service.prune_shadowed_manual_tags ? '' : 'opacity-50'}>
                <div className="flex items-start gap-2">
                    <div className="flex-1">
                        <TagListField label="Keep" values={draft} onChange={setDraft} emptyHint="Nothing extra"/>
                    </div>
                    {dirty && (
                        <div className="flex shrink-0 gap-1">
                            <Button variant="ghost" size="icon" className="h-7 w-7" onClick={() => setDraft(service.keep_manual_tags)}>
                                <Undo2 className="h-3.5 w-3.5"/>
                            </Button>
                            <Button size="icon" className="h-7 w-7" onClick={() => onUpdate({keep_manual_tags: draft})} disabled={isPending}>
                                <Save className="h-3.5 w-3.5"/>
                            </Button>
                        </div>
                    )}
                </div>
                {anchors.length > 0 && (
                    <div className="mt-2 flex flex-wrap items-center gap-1.5">
                        <span className="w-20 shrink-0 text-xs font-medium uppercase tracking-wide text-muted-foreground">
                            By gates
                        </span>
                        {anchors.map((a) => (
                            <Badge key={a} variant="outline" className="font-normal text-muted-foreground">
                                {TagPath.toDisplay(a)}
                            </Badge>
                        ))}
                    </div>
                )}
            </div>
        </div>
    )
}
