import {AlertTriangle, Plus, Sparkles, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Switch} from '@/components/ui/switch'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {TagPicker} from '@/components/tags/TagPicker'
import {TagPath} from '@/lib/utils'
import type {QueryNode, WriteBack, WriteBackOp} from '@/lib/types'

/** Build the write-back op-lists the validator expects from a query's predicate (§7.2). */
function suggestWriteBack(node: QueryNode): WriteBack {
    const include = node.include ?? []
    const exclude = node.exclude ?? []
    const onAdd: WriteBackOp[] = [
        ...include.map((path): WriteBackOp => ({op: 'assign', path})),
        ...exclude.map((path): WriteBackOp => ({op: 'remove', path})),
    ]
    let onRemove: WriteBackOp[] = []
    if (node.match === 'any') {
        onRemove = include.map((path): WriteBackOp => ({op: 'remove', path}))
    } else if (include.length > 0) {
        onRemove = [{op: 'remove', path: include[0]}]
    } else if (exclude.length > 0) {
        onRemove = [{op: 'assign', path: exclude[0]}]
    }
    return {onAdd, onRemove}
}

function OpList({
                    title,
                    ops,
                    onChange,
                }: {
    title: string
    ops: WriteBackOp[]
    onChange: (next: WriteBackOp[]) => void
}) {
    const update = (i: number, patch: Partial<WriteBackOp>) =>
        onChange(ops.map((op, idx) => (idx === i ? {...op, ...patch} : op)))

    return (
        <div className="space-y-1.5">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{title}</p>
            {ops.length === 0 && <p className="text-xs italic text-muted-foreground">No operations.</p>}
            {ops.map((op, i) => (
                <div key={i} className="flex items-center gap-1.5">
                    <Select value={op.op} onValueChange={(v) => update(i, {op: v as WriteBackOp['op']})}>
                        <SelectTrigger className="h-7 w-28 text-xs">
                            <SelectValue/>
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value="assign">assign</SelectItem>
                            <SelectItem value="remove">remove</SelectItem>
                        </SelectContent>
                    </Select>
                    <TagPicker
                        allowProtected
                        allowCreate
                        onSelect={(wire) => update(i, {path: wire})}
                        trigger={
                            <Button variant="outline" size="sm" className="h-7 flex-1 justify-start font-normal">
                                {op.path ? TagPath.toDisplay(op.path) : <span className="text-muted-foreground">Pick tag…</span>}
                            </Button>
                        }
                    />
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 text-muted-foreground"
                        onClick={() => onChange(ops.filter((_, idx) => idx !== i))}
                        aria-label="Remove operation"
                    >
                        <X className="h-3.5 w-3.5"/>
                    </Button>
                </div>
            ))}
            <Button
                variant="ghost"
                size="sm"
                className="h-7 gap-1.5 text-xs"
                onClick={() => onChange([...ops, {op: 'assign', path: ''}])}
            >
                <Plus className="h-3.5 w-3.5"/>
                Add operation
            </Button>
        </div>
    )
}

/**
 * Edits a query node's write-back op-list. Off ⇒ `null` (no op-list). On ⇒ the `onAdd`/`onRemove`
 * ops exercised when WebDAV writes land. Untagged nodes may now carry a **free-form** op-list
 * (feature 18 §6) — the predicate-based "suggest" helper is hidden there. `effectiveEnabled`
 * reflects the tri-state write-back gate (Advanced): when off, the op-list is inactive.
 */
export function WriteBackEditor({
                                    node,
                                    untagged,
                                    effectiveEnabled,
                                    onChange,
                                }: {
    node: QueryNode
    untagged: boolean
    effectiveEnabled: boolean
    onChange: (wb: WriteBack | null) => void
}) {
    const wb = node.writeBack ?? null
    const enabled = wb != null
    const hasOnAdd = !!wb && wb.onAdd.length > 0

    return (
        <div className="space-y-2 rounded-md border border-border/60 p-3">
            <div className="flex items-center justify-between">
                <div>
                    <p className="text-sm font-medium">Write-back op-list</p>
                    <p className="text-xs text-muted-foreground">
                        {untagged
                            ? 'Free-form tag ops applied on WebDAV writes into this untagged folder.'
                            : 'Makes the directory writable (used by WebDAV).'}
                    </p>
                </div>
                <Switch
                    checked={enabled}
                    onCheckedChange={(on) => onChange(on ? (untagged ? {onAdd: [], onRemove: []} : suggestWriteBack(node)) : null)}
                />
            </div>

            {enabled && !effectiveEnabled && (
                <p className="text-[11px] text-amber-600 dark:text-amber-400">
                    Write-back is off for this folder (see Advanced), so this op-list is inactive and the folder stays
                    read-only.
                </p>
            )}

            {enabled && untagged && hasOnAdd && (
                <p className="flex items-start gap-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                    <AlertTriangle className="mt-px h-3 w-3 shrink-0"/>
                    <span>
                        “On add” can’t guarantee a picture becomes untagged — a live rule/segment/share tag keeps it out
                        of this folder after the write (and may cause a conflict).
                    </span>
                </p>
            )}

            {enabled && wb && (
                <div className="space-y-3 pt-1">
                    <OpList title="On add" ops={wb.onAdd} onChange={(onAdd) => onChange({...wb, onAdd})}/>
                    <OpList title="On remove" ops={wb.onRemove} onChange={(onRemove) => onChange({...wb, onRemove})}/>
                    {!untagged && (
                        <Button
                            variant="ghost"
                            size="sm"
                            className="h-7 gap-1.5 text-xs text-muted-foreground"
                            onClick={() => onChange(suggestWriteBack(node))}
                        >
                            <Sparkles className="h-3.5 w-3.5"/>
                            Reset to suggested
                        </Button>
                    )}
                </div>
            )}
        </div>
    )
}
