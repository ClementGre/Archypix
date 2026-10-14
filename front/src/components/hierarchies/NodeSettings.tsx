import type {ReactNode} from 'react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {NumberInput} from '@/components/ui/number-input'
import {Switch} from '@/components/ui/switch'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {TagPicker} from '@/components/tags/TagPicker'
import {TagListField} from '@/components/tags/TagListField'
import {cn, TagPath} from '@/lib/utils'
import type {
    DeeperMode,
    DropNode,
    HierarchyNode,
    MirrorNode,
    NamingStrategy,
    QueryNode,
    SafeDeleteMode,
    StaticNode,
    TrashedDuplicate,
    WriteBackOp,
} from '@/lib/types'
import {WriteBackEditor} from './WriteBackEditor'
import {
    effectiveWriteBack,
    type Inherited,
    KIND_COLOR,
    KIND_HINT,
    KIND_LABEL,
    NAMING_OPTIONS,
    nodeDisplayName,
    SAFE_DELETE_OPTIONS,
    TRASHED_DUPLICATE_OPTIONS,
} from './hierarchyUtils'

const INHERIT = '__inherit__'

function Hint({children}: { children: ReactNode }) {
    return <p className="text-[11px] text-muted-foreground">{children}</p>
}

/** A labelled Select with an "Inherit (…)" first option mapping to `null`. */
function InheritSelect<V extends string>({
                                             label,
                                             value,
                                             inheritLabel,
                                             options,
                                             onChange,
                                             disabled,
                                         }: {
    label: string
    value: V | null | undefined
    inheritLabel: string
    options: { value: V; label: string }[]
    onChange: (v: V | null) => void
    disabled?: boolean
}) {
    return (
        <div className="space-y-1">
            <Label className="text-xs text-muted-foreground">{label}</Label>
            <Select value={value ?? INHERIT} disabled={disabled} onValueChange={(v) => onChange(v === INHERIT ? null : (v as V))}>
                <SelectTrigger className="h-8 text-xs">
                    <SelectValue/>
                </SelectTrigger>
                <SelectContent>
                    <SelectItem value={INHERIT}>{inheritLabel}</SelectItem>
                    {options.map((o) => (
                        <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                    ))}
                </SelectContent>
            </Select>
        </div>
    )
}

const labelOf = (v: TrashedDuplicate) => TRASHED_DUPLICATE_OPTIONS.find((o) => o.value === v)?.label ?? v

/** The `trashedDuplicate` tri-state (feature 39 §6). Moot, so disabled, where nothing can be written. */
function TrashedDuplicateSelect({node, inh, writable, onChange}: {
    node: HierarchyNode
    inh: Inherited
    writable: boolean
    onChange: (v: TrashedDuplicate | null) => void
}) {
    return (
        <InheritSelect
            label="Re-upload of a trashed photo"
            value={node.trashedDuplicate}
            inheritLabel={`Inherit (${labelOf(inh.trashedDuplicate).toLowerCase()})`}
            options={TRASHED_DUPLICATE_OPTIONS}
            disabled={!writable}
            onChange={onChange}
        />
    )
}

/** Per-node write-back and trashed-duplicate tri-states, plus naming and safe-delete overrides. */
function AdvancedSection<T extends HierarchyNode>({
                                                      node,
                                                      onChange,
                                                      inh,
                                                      isStatic = false,
                                                  }: {
    node: T
    onChange: (n: T) => void
    inh: Inherited
    isStatic?: boolean
}) {
    const set = (patch: Partial<T>) => onChange({...node, ...patch})
    const eff = effectiveWriteBack(inh.master, inh.writeBack, node.writeBackEnabled)
    const wbValue = node.writeBackEnabled == null ? INHERIT : node.writeBackEnabled ? 'on' : 'off'

    return (
        <section className="space-y-3 border-t pt-4">
            <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Advanced</h3>
            <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Write-back</Label>
                    <Select
                        value={wbValue}
                        disabled={!inh.master}
                        onValueChange={(v) => set({writeBackEnabled: v === INHERIT ? undefined : v === 'on'} as Partial<T>)}
                    >
                        <SelectTrigger className="h-8 text-xs">
                            <SelectValue/>
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value={INHERIT}>Inherit ({inh.master && inh.writeBack ? 'on' : 'off'})</SelectItem>
                            <SelectItem value="on">On</SelectItem>
                            <SelectItem value="off">Off</SelectItem>
                        </SelectContent>
                    </Select>
                </div>
                <TrashedDuplicateSelect
                    node={node}
                    inh={inh}
                    writable={eff}
                    onChange={(v) => set({trashedDuplicate: v} as Partial<T>)}
                />
            </div>
            <Hint>
                {!inh.master
                    ? 'Write-back is off for the whole hierarchy. Every folder here is read-only.'
                    : isStatic
                        ? 'A static folder is never written into. These values are the default for its sub-folders.'
                        : 'Both apply to this folder and its sub-folders.'}
            </Hint>
            <div className="grid grid-cols-2 gap-3">
                <InheritSelect
                    label="Naming"
                    value={node.naming}
                    inheritLabel="Inherit"
                    options={NAMING_OPTIONS}
                    onChange={(v: NamingStrategy | null) => set({naming: v} as Partial<T>)}
                />
                {/* A read-only folder always moves deleted pictures to the trash (18 §5.3). */}
                <InheritSelect
                    label="Safe delete"
                    value={eff ? node.safeDeleteMode : 'fullDelete'}
                    inheritLabel="Inherit"
                    options={SAFE_DELETE_OPTIONS}
                    disabled={!eff}
                    onChange={(v: SafeDeleteMode | null) => set({safeDeleteMode: v} as Partial<T>)}
                />
            </div>
        </section>
    )
}

const DEEPER_OPTIONS: { value: DeeperMode; label: string }[] = [
    {value: 'collapse', label: 'Show them in the deepest folder'},
    {value: 'exclude', label: 'Hide them'},
]

function DepthInput({value, min, onChange}: { value: number; min: number; onChange: (n: number) => void }) {
    return (
        <NumberInput
            className="h-8"
            min={min}
            step={1}
            value={value}
            onChange={(e) => onChange(Math.max(min, Math.floor(Number(e.target.value) || 0)))}
        />
    )
}

function MirrorFields({node, onChange, inh}: { node: MirrorNode; onChange: (n: HierarchyNode) => void; inh: Inherited }) {
    const set = (patch: Partial<MirrorNode>) => onChange({...node, ...patch})
    const maxDepth = node.maxDepth ?? 0
    // Level 1 is the tag root's children; `keepDir` alone decides whether the root itself shows.
    const minDepth = Math.max(1, node.minDepth ?? 1)
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
                <span>Keep <code className="text-xs">{TagPath.leaf(node.tagRoot) || 'root'}</code> as a folder</span>
            </label>
            <TagListField
                label="Collapsed"
                values={node.collapsed ?? []}
                onChange={(v) => set({collapsed: v.length ? v : undefined})}
                emptyHint="None"
            />
            <TagListField
                label="Included"
                values={node.include ?? []}
                onChange={(v) => set({include: v.length ? v : undefined})}
                color="emerald"
                emptyHint="None"
            />
            <TagListField
                label="Excluded"
                values={node.exclude ?? []}
                onChange={(v) => set({exclude: v.length ? v : undefined})}
                color="red"
                emptyHint="None"
            />
            <Hint>
                A collapsed tag shows its pictures in its parent folder. It must be under the tag root. An included
                tag is required on every picture, and added to pictures written here. An excluded tag under the tag
                root removes that folder. Any other excluded tag hides its pictures. Empty folders are hidden.
            </Hint>

            {/* Depth band (18 §7, 39 §5.2). */}
            <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Min depth</Label>
                    <DepthInput value={minDepth} min={1} onChange={(n) => set({minDepth: n > 1 ? n : undefined})}/>
                    <Hint>First folder level shown. 1 is the tag root's sub-tags. Pictures tagged higher are hidden.</Hint>
                </div>
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Max depth</Label>
                    <DepthInput value={maxDepth} min={0} onChange={(n) => set({maxDepth: n || undefined})}/>
                    <Hint>Last folder level shown. 0 means no limit.</Hint>
                </div>
            </div>
            {maxDepth >= 1 && (
                <div className="space-y-1">
                    <Label className="text-xs text-muted-foreground">Pictures below the max depth</Label>
                    <Select value={node.deeperMode ?? 'collapse'} onValueChange={(v) => set({deeperMode: v as DeeperMode})}>
                        <SelectTrigger className="h-8 text-xs">
                            <SelectValue/>
                        </SelectTrigger>
                        <SelectContent>
                            {DEEPER_OPTIONS.map((o) => (
                                <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                            ))}
                        </SelectContent>
                    </Select>
                </div>
            )}

            <AdvancedSection node={node} onChange={onChange} inh={inh}/>
        </div>
    )
}

function QueryFields({node, onChange, inh}: { node: QueryNode; onChange: (n: HierarchyNode) => void; inh: Inherited }) {
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
                    <Select value={node.match ?? 'all'} disabled={untagged} onValueChange={(v) => set({match: v as 'all' | 'any'})}>
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
                        // Untagged requires empty include/exclude, but may keep a free-form op-list (18 §6).
                        set(v ? {matchUntagged: true, include: [], exclude: []} : {matchUntagged: false})
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
                </>
            )}
            <WriteBackEditor
                node={node}
                untagged={untagged}
                effectiveEnabled={effectiveWriteBack(inh.master, inh.writeBack, node.writeBackEnabled)}
                onChange={(wbOp) => set({writeBack: wbOp})}
            />

            <AdvancedSection node={node} onChange={onChange} inh={inh}/>
        </div>
    )
}

function StaticFields({node, onChange, inh}: { node: StaticNode; onChange: (n: HierarchyNode) => void; inh: Inherited }) {
    return (
        <div className="space-y-3">
            <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">Name</Label>
                <Input className="h-8" value={node.name} onChange={(e) => onChange({...node, name: e.target.value})}/>
            </div>
            <AdvancedSection node={node} onChange={onChange} inh={inh} isStatic/>
        </div>
    )
}

/** Upload inbox (feature 18 §4): tags every upload, lists recent ones, always writable. */
function DropFields({node, onChange, inh}: { node: DropNode; onChange: (n: HierarchyNode) => void; inh: Inherited }) {
    const set = (patch: Partial<DropNode>) => onChange({...node, ...patch})
    const assignPaths = node.onAdd.filter((o) => o.op === 'assign').map((o) => o.path)
    const otherOps = node.onAdd.filter((o) => o.op !== 'assign')
    const setPaths = (paths: string[]) =>
        set({onAdd: [...paths.map((path): WriteBackOp => ({op: 'assign', path})), ...otherOps]})

    return (
        <div className="space-y-3">
            <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">Name</Label>
                <Input className="h-8" value={node.name} onChange={(e) => set({name: e.target.value})}/>
            </div>
            <TagListField
                label="Assign"
                values={assignPaths}
                onChange={setPaths}
                color="emerald"
                emptyHint="No tags. Uploads land untagged."
            />
            <Hint>
                Every upload gets these tags. The folder lists only the last few minutes of uploads, so sync apps can
                check them. It is always writable, even when write-back is off.
            </Hint>
            <TrashedDuplicateSelect node={node} inh={inh} writable onChange={(v) => set({trashedDuplicate: v})}/>
        </div>
    )
}

/** The selected node's settings: the detail half of the hierarchy editor (feature 39 §9). */
export function NodeSettings({
                                 node,
                                 onChange,
                                 inherited,
                             }: {
    node: HierarchyNode
    onChange: (n: HierarchyNode) => void
    inherited: Inherited
}) {
    return (
        <div className="space-y-4">
            <div className="flex items-center gap-2">
                <Badge variant="secondary" className={cn('border-0 font-medium', KIND_COLOR[node.kind])}>
                    {KIND_LABEL[node.kind]}
                </Badge>
                <span className="truncate text-sm font-medium">{nodeDisplayName(node)}</span>
            </div>
            <p className="text-xs text-muted-foreground">{KIND_HINT[node.kind]}</p>
            {node.kind === 'mirror' && <MirrorFields node={node} onChange={onChange} inh={inherited}/>}
            {node.kind === 'query' && <QueryFields node={node} onChange={onChange} inh={inherited}/>}
            {node.kind === 'static' && <StaticFields node={node} onChange={onChange} inh={inherited}/>}
            {node.kind === 'drop' && <DropFields node={node} onChange={onChange} inh={inherited}/>}
        </div>
    )
}
