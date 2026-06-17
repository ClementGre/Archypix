import {Plus, Sparkles, X} from 'lucide-react'
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
 * Edits a query node's write-back. Off ⇒ `null` (read-only directory). On ⇒ the
 * `onAdd`/`onRemove` op-lists exercised when WebDAV writes land. The webapp
 * navigation is read-only, so this is forward-looking config.
 */
export function WriteBackEditor({node, onChange}: { node: QueryNode; onChange: (wb: WriteBack | null) => void }) {
    const wb = node.writeBack ?? null
    const enabled = wb != null
    const disabledByUntagged = !!node.matchUntagged

    return (
        <div className="space-y-2 rounded-md border border-border/60 p-3">
            <div className="flex items-center justify-between">
                <div>
                    <p className="text-sm font-medium">Write-back</p>
                    <p className="text-xs text-muted-foreground">
                        {disabledByUntagged
                            ? 'Untagged directories are always read-only.'
                            : 'Makes the directory writable (used by WebDAV).'}
                    </p>
                </div>
                <Switch
                    checked={enabled}
                    disabled={disabledByUntagged}
                    onCheckedChange={(on) => onChange(on ? suggestWriteBack(node) : null)}
                />
            </div>

            {enabled && wb && (
                <div className="space-y-3 pt-1">
                    <OpList title="On add" ops={wb.onAdd} onChange={(onAdd) => onChange({...wb, onAdd})}/>
                    <OpList title="On remove" ops={wb.onRemove} onChange={(onRemove) => onChange({...wb, onRemove})}/>
                    <Button
                        variant="ghost"
                        size="sm"
                        className="h-7 gap-1.5 text-xs text-muted-foreground"
                        onClick={() => onChange(suggestWriteBack(node))}
                    >
                        <Sparkles className="h-3.5 w-3.5"/>
                        Reset to suggested
                    </Button>
                </div>
            )}
        </div>
    )
}
