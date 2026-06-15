import {useState} from 'react'
import {AlertTriangle, Trash2} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Badge} from '@/components/ui/badge'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {TagPicker} from '@/components/tags/TagPicker'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useIncomingShares} from '@/hooks/useShares'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {TagPath} from '@/lib/utils'
import {apiErrorMessage} from '@/api/client'
import type {SharedTagMappingRule} from '@/lib/types'

interface MappingEditorProps {
    serviceId: string
    mappings: SharedTagMappingRule[]
}

export function MappingEditor({serviceId, mappings}: MappingEditorProps) {
    const {data: shares} = useIncomingShares()
    const {addMapping, deleteMapping} = useTaggingMutations()

    const [selectedShareId, setSelectedShareId] = useState<string>('')
    const [assignTag, setAssignTag] = useState<string>('')

    const handleAdd = () => {
        if (!selectedShareId || !assignTag) return
        addMapping.mutate(
            {serviceId, incoming_share_id: selectedShareId, assign_tag: assignTag},
            {
                onSuccess: () => {
                    setSelectedShareId('')
                    setAssignTag('')
                },
                onError: (err) => toast.error(apiErrorMessage(err)),
            },
        )
    }

    const handleDelete = (ruleId: string) => {
        deleteMapping.mutate({serviceId, ruleId}, {onError: (err) => toast.error(apiErrorMessage(err))})
    }

    // A share may carry a single mapping — only offer shares not already mapped here.
    const mappedShareIds = new Set(mappings.map((m) => m.incoming_share_id))
    const selectableShares = (shares ?? []).filter((s) => s.status === 'active' && !mappedShareIds.has(s.id))

    return (
        <div className="space-y-3">
            {mappings.length === 0 && <p className="text-sm text-muted-foreground">No mappings yet.</p>}
            <div className="space-y-1.5">
                {mappings.map((m) => {
                    const share = (shares ?? []).find((s) => s.id === m.incoming_share_id)
                    return (
                        <div key={m.id} className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm">
              <span className="flex-1 font-mono text-xs text-muted-foreground">
                {share ? `@${share.sender_username}:${share.sender_instance}` : m.incoming_share_id}
              </span>
                            <span className="text-muted-foreground">→</span>
                            <span className="flex-1">{TagPath.toDisplay(m.assign_tag)}</span>
                            {m.is_broken && (
                                <Badge variant="secondary" className="gap-1 border-0 bg-red-500/15 text-red-500">
                                    <AlertTriangle className="h-3 w-3"/>
                                    broken
                                </Badge>
                            )}
                            <ConfirmDialog
                                title="Remove mapping?"
                                description="The local tag assigned by this mapping will be removed from the shared pictures."
                                confirmLabel="Remove"
                                destructive
                                onConfirm={() => handleDelete(m.id)}
                                trigger={
                                    <Button
                                        variant="ghost"
                                        size="icon"
                                        className="h-6 w-6 text-muted-foreground hover:text-destructive"
                                        disabled={deleteMapping.isPending}
                                    >
                                        <Trash2 className="h-3.5 w-3.5"/>
                                    </Button>
                                }
                            />
                        </div>
                    )
                })}
            </div>

            {/* Add mapping form */}
            <div className="flex flex-wrap items-end gap-2 rounded-md border border-dashed p-3">
                <div className="min-w-40 flex-1">
                    <label className="mb-1 block text-xs text-muted-foreground">Incoming share</label>
                    <Select value={selectedShareId} onValueChange={setSelectedShareId}>
                        <SelectTrigger className="h-8 text-sm">
                            <SelectValue placeholder="Select share…"/>
                        </SelectTrigger>
                        <SelectContent>
                            {selectableShares.map((s) => (
                                <SelectItem key={s.id} value={s.id}>
                                    @{s.sender_username}:{s.sender_instance}
                                </SelectItem>
                            ))}
                            {selectableShares.length === 0 && (
                                <SelectItem value="__none__" disabled>
                                    No unmapped active shares
                                </SelectItem>
                            )}
                        </SelectContent>
                    </Select>
                </div>
                <div className="min-w-40 flex-1">
                    <label className="mb-1 block text-xs text-muted-foreground">Assign tag</label>
                    <div className="flex items-center gap-1.5">
                        {assignTag && <span className="text-sm text-foreground">{TagPath.toDisplay(assignTag)}</span>}
                        <TagPicker onSelect={setAssignTag} allowCreate triggerLabel={assignTag ? 'Change' : 'Pick tag'}/>
                    </div>
                </div>
                <Button size="sm" onClick={handleAdd} disabled={!selectedShareId || !assignTag || addMapping.isPending}>
                    Add mapping
                </Button>
            </div>
        </div>
    )
}
