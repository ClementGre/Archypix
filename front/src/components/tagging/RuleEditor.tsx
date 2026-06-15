import {useState} from 'react'
import {Trash2} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {TagPicker} from '@/components/tags/TagPicker'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {TagPath} from '@/lib/utils'
import {apiErrorMessage} from '@/api/client'
import type {RuleTaggingRule} from '@/lib/types'

const PREDICATE_HELP = [
    'gps_within_bbox(lat_min, lat_max, lon_min, lon_max)',
    'capture_year(YYYY)',
    'capture_month(M)',
    'filename_contains("string")',
]

interface RuleEditorProps {
    serviceId: string
    rules: RuleTaggingRule[]
}

export function RuleEditor({serviceId, rules}: RuleEditorProps) {
    const {addRule, deleteRule} = useTaggingMutations()
    const [predicate, setPredicate] = useState('')
    const [assignTag, setAssignTag] = useState('')

    const handleAdd = () => {
        if (!predicate.trim() || !assignTag) return
        addRule.mutate(
            {serviceId, predicate: predicate.trim(), assign_tag: assignTag},
            {
                onSuccess: () => {
                    setPredicate('')
                    setAssignTag('')
                },
                onError: (err) => toast.error(apiErrorMessage(err)),
            },
        )
    }

    const handleDelete = (ruleId: string) => {
        deleteRule.mutate(
            {serviceId, ruleId},
            {onError: (err) => toast.error(apiErrorMessage(err))},
        )
    }

    return (
        <div className="space-y-3">
            {rules.length === 0 && (
                <p className="text-sm text-muted-foreground">No rules yet.</p>
            )}
            <div className="space-y-1.5">
                {rules.map((r) => (
                    <div key={r.id} className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm">
                        <code className="flex-1 font-mono text-xs">{r.predicate}</code>
                        <span className="text-muted-foreground">→</span>
                        <span className="flex-1">{TagPath.toDisplay(r.assign_tag)}</span>
                        <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6 text-muted-foreground hover:text-destructive"
                            onClick={() => handleDelete(r.id)}
                            disabled={deleteRule.isPending}
                        >
                            <Trash2 className="h-3.5 w-3.5"/>
                        </Button>
                    </div>
                ))}
            </div>

            {/* Add rule form */}
            <div className="rounded-md border border-dashed p-3 space-y-2">
                <div>
                    <label className="mb-1 block text-xs text-muted-foreground">Predicate</label>
                    <Input
                        value={predicate}
                        onChange={(e) => setPredicate(e.target.value)}
                        placeholder="e.g. capture_year(2024)"
                        className="h-8 font-mono text-sm"
                    />
                    <p className="mt-1 text-xs text-muted-foreground">
                        Supported forms:{' '}
                        {PREDICATE_HELP.map((h, i) => (
                            <span key={h}>
                                <code className="rounded bg-muted px-0.5">{h}</code>
                                {i < PREDICATE_HELP.length - 1 && ', '}
                            </span>
                        ))}
                    </p>
                </div>
                <div className="flex flex-wrap items-end gap-2">
                    <div>
                        <label className="mb-1 block text-xs text-muted-foreground">Assign tag</label>
                        <div className="flex items-center gap-1.5">
                            {assignTag && (
                                <span className="text-sm">{TagPath.toDisplay(assignTag)}</span>
                            )}
                            <TagPicker
                                onSelect={setAssignTag}
                                allowCreate={true}
                                triggerLabel={assignTag ? 'Change' : 'Pick tag'}
                            />
                        </div>
                    </div>
                    <Button
                        size="sm"
                        onClick={handleAdd}
                        disabled={!predicate.trim() || !assignTag || addRule.isPending}
                    >
                        Add rule
                    </Button>
                </div>
            </div>
        </div>
    )
}
