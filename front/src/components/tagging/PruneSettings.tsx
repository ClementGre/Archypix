import {Checkbox} from '@/components/ui/checkbox'
import {TagListField} from '@/components/tags/TagListField'

/** Shadowed manual tag pruning (feature 39 §4): the opt-in and the keep-list floor. */
export function PruneSettings({
                                  prune,
                                  keep,
                                  onChange,
                              }: {
    prune: boolean
    keep: string[]
    onChange: (patch: { prune_shadowed_manual_tags?: boolean; keep_manual_tags?: string[] }) => void
}) {
    return (
        <div className="space-y-3 text-sm">
            <label className="flex items-start gap-2">
                <Checkbox
                    checked={prune}
                    onCheckedChange={(v) => onChange({prune_shadowed_manual_tags: v === true})}
                    className="mt-0.5"
                />
                <span>
                    Remove my manual tags that this service also adds
                    <span className="block text-xs text-muted-foreground">
                        The service then owns the tag. A manual copy, for example from a WebDAV move, shrinks to what
                        the services' conditions still need.
                    </span>
                </span>
            </label>
            <div className={prune ? '' : 'opacity-50'}>
                <TagListField
                    label="Keep"
                    values={keep}
                    onChange={(v) => onChange({keep_manual_tags: v})}
                    emptyHint="Nothing extra"
                />
                <p className="mt-1 text-[11px] text-muted-foreground">Never shrink a manual tag below these tags.</p>
            </div>
        </div>
    )
}
