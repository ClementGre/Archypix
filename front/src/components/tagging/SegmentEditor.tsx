import {useState} from 'react'
import {Trash2} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {TagPicker} from '@/components/tags/TagPicker'
import {DateRangePicker} from '@/components/common/DateRangePicker'
import {useTaggingMutations} from '@/hooks/useTaggingServices'
import {TagPath} from '@/lib/utils'
import {apiErrorMessage} from '@/api/client'
import type {SegmentationSegment} from '@/lib/types'

// Radix Select forbids an empty-string item value, so the "no parent" choice uses a sentinel
// that maps back to '' (top-level segment).
const NONE_PARENT = '__none__'

interface SegmentEditorProps {
    serviceId: string
    segments: SegmentationSegment[]
}

export function SegmentEditor({serviceId, segments}: SegmentEditorProps) {
    const {addSegment, deleteSegment} = useTaggingMutations()

    const [name, setName] = useState('')
    const [dateStart, setDateStart] = useState('')
    const [dateEnd, setDateEnd] = useState('')
    const [assignTag, setAssignTag] = useState('')
    const [parentId, setParentId] = useState<string>('')
    const [dateError, setDateError] = useState('')

    const validate = () => {
        if (!dateStart || !dateEnd) return false
        // Naive strings of the same format compare correctly lexicographically.
        if (dateEnd <= dateStart) {
            setDateError('End must be after start.')
            return false
        }
        setDateError('')
        return true
    }

    const handleAdd = () => {
        if (!name.trim() || !assignTag || !validate()) return
        addSegment.mutate(
            {
                serviceId,
                name: name.trim(),
                // Naive datetimes (no timezone) — the backend deserializes a NaiveDateTime.
                date_start: dateStart,
                date_end: dateEnd,
                assign_tag: assignTag,
                parent_segment_id: parentId || undefined,
            },
            {
                onSuccess: () => {
                    setName('')
                    setDateStart('')
                    setDateEnd('')
                    setAssignTag('')
                    setParentId('')
                    setDateError('')
                },
                onError: (err) => toast.error(apiErrorMessage(err)),
            },
        )
    }

    return (
        <div className="space-y-3">
            {segments.length === 0 && (
                <p className="text-sm text-muted-foreground">No segments yet.</p>
            )}
            <div className="space-y-1.5">
                {segments.map((seg) => (
                    <div
                        key={seg.id}
                        className={`flex items-center gap-2 rounded-md border px-3 py-2 text-sm ${seg.parent_segment_id ? 'ml-6 border-l-2 border-l-muted-foreground/30' : ''}`}
                    >
                        <span className="font-medium flex-shrink-0">{seg.name}</span>
                        <span className="text-muted-foreground text-xs flex-1">
                            {new Date(seg.date_start).toLocaleDateString()} –{' '}
                            {new Date(seg.date_end).toLocaleDateString()}
                        </span>
                        <span className="text-muted-foreground">→</span>
                        <span className="flex-1">{TagPath.toDisplay(seg.assign_tag)}</span>
                        <Button
                            variant="ghost"
                            size="icon"
                            className="h-6 w-6 text-muted-foreground hover:text-destructive"
                            onClick={() =>
                                deleteSegment.mutate(
                                    {serviceId, segmentId: seg.id},
                                    {onError: (err) => toast.error(apiErrorMessage(err))},
                                )
                            }
                            disabled={deleteSegment.isPending}
                        >
                            <Trash2 className="h-3.5 w-3.5"/>
                        </Button>
                    </div>
                ))}
            </div>

            {/* Add segment form */}
            <div className="rounded-md border border-dashed p-3 space-y-2">
                <div className="grid grid-cols-2 gap-2">
                    <div>
                        <label className="mb-1 block text-xs text-muted-foreground">Name</label>
                        <Input
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            placeholder="e.g. Summer 2024"
                            className="h-8 text-sm"
                        />
                    </div>
                    <div>
                        <label className="mb-1 block text-xs text-muted-foreground">Parent segment (optional)</label>
                        <Select
                            value={parentId || NONE_PARENT}
                            onValueChange={(v) => setParentId(v === NONE_PARENT ? '' : v)}
                        >
                            <SelectTrigger className="h-8 text-sm">
                                <SelectValue placeholder="None (top-level)"/>
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={NONE_PARENT}>None (top-level)</SelectItem>
                                {segments
                                    .filter((s) => !s.parent_segment_id)
                                    .map((s) => (
                                        <SelectItem key={s.id} value={s.id}>
                                            {s.name}
                                        </SelectItem>
                                    ))}
                            </SelectContent>
                        </Select>
                    </div>
                </div>
                <div>
                    <label className="mb-1 block text-xs text-muted-foreground">Date range</label>
                    <DateRangePicker
                        mode="datetime"
                        from={dateStart}
                        to={dateEnd}
                        onChange={(f, t) => {
                            setDateStart(f)
                            setDateEnd(t)
                            setDateError('')
                        }}
                        placeholder="Pick start and end days"
                    />
                </div>
                {dateError && <p className="text-xs text-destructive">{dateError}</p>}
                <div className="flex flex-wrap items-end gap-2">
                    <div>
                        <label className="mb-1 block text-xs text-muted-foreground">Assign tag</label>
                        <div className="flex items-center gap-1.5">
                            {assignTag && <span className="text-sm">{TagPath.toDisplay(assignTag)}</span>}
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
                        disabled={!name.trim() || !dateStart || !dateEnd || !assignTag || addSegment.isPending}
                    >
                        Add segment
                    </Button>
                </div>
            </div>
        </div>
    )
}
