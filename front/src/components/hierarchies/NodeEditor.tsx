import {useState} from 'react'
import {ChevronDown, ChevronUp, FolderPlus, Settings2, Trash2} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,} from '@/components/ui/dropdown-menu'
import {TagPicker} from '@/components/tags/TagPicker'
import {cn, TagPath} from '@/lib/utils'
import type {HierarchyNode, MirrorNode, NamingStrategy, NodeKind, QueryNode, SafeDeleteMode, StaticNode,} from '@/lib/types'
import {TagListField} from './TagListField'
import {WriteBackEditor} from './WriteBackEditor'
import {KIND_COLOR, KIND_HINT, KIND_LABEL, makeNode, NAMING_OPTIONS, nodeDisplayName, SAFE_DELETE_OPTIONS,} from './hierarchyUtils'

const INHERIT = '__inherit__'

/** Per-node naming / safe-delete overrides (inherit from the hierarchy default by default). */
function CommonAdvanced<T extends HierarchyNode>({node, onChange}: { node: T; onChange: (n: T) => void }) {
    const [open, setOpen] = useState(false)
    const set = (patch: Partial<T>) => onChange({...node, ...patch})

    return (
        <div>
            <Button
                variant="ghost"
                size="sm"
                className="h-7 gap-1.5 px-1 text-xs text-muted-foreground"
                onClick={() => setOpen((o) => !o)}
            >
                <Settings2 className="h-3.5 w-3.5"/>
                Advanced
            </Button>
            {open && (
                <div className="grid grid-cols-2 gap-3 pt-1">
                    <div className="space-y-1">
                        <Label className="text-xs text-muted-foreground">Naming</Label>
                        <Select
                            value={node.naming ?? INHERIT}
                            onValueChange={(v) => set({naming: v === INHERIT ? null : (v as NamingStrategy)} as Partial<T>)}
                        >
                            <SelectTrigger className="h-8 text-xs">
                                <SelectValue/>
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={INHERIT}>Inherit</SelectItem>
                                {NAMING_OPTIONS.map((o) => (
                                    <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                    </div>
                    <div className="space-y-1">
                        <Label className="text-xs text-muted-foreground">Safe delete</Label>
                        <Select
                            value={node.safeDeleteMode ?? INHERIT}
                            onValueChange={(v) =>
                                set({safeDeleteMode: v === INHERIT ? null : (v as SafeDeleteMode)} as Partial<T>)
                            }
                        >
                            <SelectTrigger className="h-8 text-xs">
                                <SelectValue/>
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value={INHERIT}>Inherit</SelectItem>
                                {SAFE_DELETE_OPTIONS.map((o) => (
                                    <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                ))}
                            </SelectContent>
                        </Select>
                    </div>
                </div>
            )}
        </div>
    )
}

function MirrorFields({node, onChange}: { node: MirrorNode; onChange: (n: HierarchyNode) => void }) {
    const set = (patch: Partial<MirrorNode>) => onChange({...node, ...patch})
    return (
        <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Name (optional)</Label>
                    <Input
                        className="h-8"
                        value={node.name ?? ''}
                        placeholder={nodeDisplayName(node)}
                        onChange={(e) => set({name: e.target.value || undefined})}
                    />
                </div>
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Tag root</Label>
                    <TagPicker
                        allowProtected
                        allowCreate
                        onSelect={(wire) => set({tagRoot: wire})}
                        trigger={
                            <Button variant="outline" size="sm" className="h-8 w-full justify-start font-normal">
                                {node.tagRoot ? TagPath.toDisplay(node.tagRoot) : <span className="text-muted-foreground">Pick tag…</span>}
                            </Button>
                        }
                    />
                </div>
            </div>
            <label className="flex items-center gap-2 text-sm">
                <Switch checked={!!node.keepDir} onCheckedChange={(v) => set({keepDir: v})}/>
                <span>Keep <code className="text-xs">{TagPath.leaf(node.tagRoot) || 'root'}</code> as a directory level</span>
            </label>
            <TagListField
                label="Collapsed"
                values={node.collapsed ?? []}
                onChange={(v) => set({collapsed: v.length ? v : undefined})}
                emptyHint="None — sub-tags become their own folders"
            />
            <TagListField
                label="Excluded"
                values={node.exclude ?? []}
                onChange={(v) => set({exclude: v.length ? v : undefined})}
                color="red"
                emptyHint="None"
            />
            <p className="text-[11px] text-muted-foreground">
                Collapsed and excluded paths must be under the tag root.
            </p>
            <CommonAdvanced node={node} onChange={onChange}/>
        </div>
    )
}

function QueryFields({node, onChange, depth}: { node: QueryNode; onChange: (n: HierarchyNode) => void; depth: number }) {
    const set = (patch: Partial<QueryNode>) => onChange({...node, ...patch})
    const untagged = !!node.matchUntagged
    return (
        <div className="space-y-3">
            <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Name</Label>
                    <Input className="h-8" value={node.name} onChange={(e) => set({name: e.target.value})}/>
                </div>
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Match</Label>
                    <Select
                        value={node.match ?? 'all'}
                        disabled={untagged}
                        onValueChange={(v) => set({match: v as 'all' | 'any'})}
                    >
                        <SelectTrigger className="h-8 text-xs">
                            <SelectValue/>
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value="all">All (AND)</SelectItem>
                            <SelectItem value="any">Any (OR)</SelectItem>
                        </SelectContent>
                    </Select>
                </div>
            </div>

            <label className="flex items-center gap-2 text-sm">
                <Switch
                    checked={untagged}
                    onCheckedChange={(v) =>
                        set(v ? {matchUntagged: true, include: [], exclude: [], writeBack: null} : {matchUntagged: false})
                    }
                />
                <span>Match untagged pictures only</span>
            </label>

            {!untagged && (
                <>
                    <TagListField
                        label="Include"
                        values={node.include ?? []}
                        onChange={(v) => set({include: v})}
                        color="emerald"
                        emptyHint="Empty matches all pictures"
                    />
                    <TagListField
                        label="Exclude"
                        values={node.exclude ?? []}
                        onChange={(v) => set({exclude: v.length ? v : undefined})}
                        color="red"
                        emptyHint="None"
                    />
                    <WriteBackEditor node={node} onChange={(wb) => set({writeBack: wb})}/>
                </>
            )}

            <CommonAdvanced node={node} onChange={onChange}/>

            <ChildrenSection
                label="Sub-folders"
                children={node.children ?? []}
                onChange={(children) => set({children: children.length ? children : undefined})}
                depth={depth}
            />
        </div>
    )
}

function StaticFields({node, onChange, depth}: { node: StaticNode; onChange: (n: HierarchyNode) => void; depth: number }) {
    const set = (patch: Partial<StaticNode>) => onChange({...node, ...patch})
    return (
        <div className="space-y-3">
            <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">Name</Label>
                <Input className="h-8" value={node.name} onChange={(e) => set({name: e.target.value})}/>
            </div>
            <CommonAdvanced node={node} onChange={onChange}/>
            <ChildrenSection
                label="Sub-folders"
                children={node.children ?? []}
                onChange={(children) => set({children: children.length ? children : undefined})}
                depth={depth}
            />
        </div>
    )
}

function ChildrenSection({
                             label,
                             children,
                             onChange,
                             depth,
                         }: {
    label: string
    children: HierarchyNode[]
    onChange: (nodes: HierarchyNode[]) => void
    depth: number
}) {
    return (
        <div className="space-y-2 border-l-2 border-border/60 pl-3">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</p>
            <NodeListEditor nodes={children} onChange={onChange} depth={depth + 1}/>
        </div>
    )
}

function NodeEditor({
                        node,
                        onChange,
                        onRemove,
                        onMoveUp,
                        onMoveDown,
                        canMoveUp,
                        canMoveDown,
                        depth,
                    }: {
    node: HierarchyNode
    onChange: (n: HierarchyNode) => void
    onRemove: () => void
    onMoveUp: () => void
    onMoveDown: () => void
    canMoveUp: boolean
    canMoveDown: boolean
    depth: number
}) {
    const [expanded, setExpanded] = useState(true)

    return (
        <div className="rounded-lg border bg-card">
            <div className="flex items-center gap-2 px-3 py-2">
                <button
                    onClick={() => setExpanded((e) => !e)}
                    className="flex h-5 w-5 shrink-0 items-center justify-center text-muted-foreground"
                    aria-label={expanded ? 'Collapse' : 'Expand'}
                >
                    <ChevronDown className={cn('h-4 w-4 transition-transform', !expanded && '-rotate-90')}/>
                </button>
                <Badge variant="secondary" className={cn('border-0 font-medium', KIND_COLOR[node.kind])}>
                    {KIND_LABEL[node.kind]}
                </Badge>
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{nodeDisplayName(node)}</span>
                <div className="flex shrink-0 items-center">
                    <Button
                        variant="ghost" size="icon" className="h-7 w-7 text-muted-foreground"
                        disabled={!canMoveUp} onClick={onMoveUp} aria-label="Move up"
                    >
                        <ChevronUp className="h-4 w-4"/>
                    </Button>
                    <Button
                        variant="ghost" size="icon" className="h-7 w-7 text-muted-foreground"
                        disabled={!canMoveDown} onClick={onMoveDown} aria-label="Move down"
                    >
                        <ChevronDown className="h-4 w-4"/>
                    </Button>
                    <Button
                        variant="ghost" size="icon" className="h-7 w-7 text-muted-foreground hover:text-destructive"
                        onClick={onRemove} aria-label="Delete node"
                    >
                        <Trash2 className="h-4 w-4"/>
                    </Button>
                </div>
            </div>
            {expanded && (
                <div className="border-t px-3 py-3">
                    <p className="mb-3 text-xs text-muted-foreground">{KIND_HINT[node.kind]}</p>
                    {node.kind === 'mirror' && <MirrorFields node={node} onChange={onChange}/>}
                    {node.kind === 'query' && <QueryFields node={node} onChange={onChange} depth={depth}/>}
                    {node.kind === 'static' && <StaticFields node={node} onChange={onChange} depth={depth}/>}
                </div>
            )}
        </div>
    )
}

/** Ordered editor for an array of nodes: edit / reorder / remove + add of any kind. */
export function NodeListEditor({
                                   nodes,
                                   onChange,
                                   depth = 0,
                               }: {
    nodes: HierarchyNode[]
    onChange: (nodes: HierarchyNode[]) => void
    depth?: number
}) {
    const replaceAt = (i: number, n: HierarchyNode) => onChange(nodes.map((x, idx) => (idx === i ? n : x)))
    const removeAt = (i: number) => onChange(nodes.filter((_, idx) => idx !== i))
    const move = (i: number, dir: -1 | 1) => {
        const j = i + dir
        if (j < 0 || j >= nodes.length) return
        const next = [...nodes]
        ;[next[i], next[j]] = [next[j], next[i]]
        onChange(next)
    }
    const add = (kind: NodeKind) => onChange([...nodes, makeNode(kind)])

    return (
        <div className="space-y-2">
            {nodes.map((node, i) => (
                <NodeEditor
                    key={node.id}
                    node={node}
                    depth={depth}
                    onChange={(n) => replaceAt(i, n)}
                    onRemove={() => removeAt(i)}
                    onMoveUp={() => move(i, -1)}
                    onMoveDown={() => move(i, 1)}
                    canMoveUp={i > 0}
                    canMoveDown={i < nodes.length - 1}
                />
            ))}

            <DropdownMenu>
                <DropdownMenuTrigger asChild>
                    <Button variant="outline" size="sm" className="gap-1.5">
                        <FolderPlus className="h-4 w-4"/>
                        Add directory
                    </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="w-64">
                    {(['mirror', 'query', 'static'] as NodeKind[]).map((kind) => (
                        <DropdownMenuItem key={kind} onClick={() => add(kind)} className="flex-col items-start gap-0.5">
                            <span className="font-medium">{KIND_LABEL[kind]}</span>
                            <span className="text-xs text-muted-foreground">{KIND_HINT[kind]}</span>
                        </DropdownMenuItem>
                    ))}
                </DropdownMenuContent>
            </DropdownMenu>
        </div>
    )
}
