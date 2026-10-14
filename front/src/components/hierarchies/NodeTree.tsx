import type {ReactNode} from 'react'
import {ChevronDown, ChevronUp, FolderPlus, Plus, Trash2} from 'lucide-react'
import {Badge} from '@/components/ui/badge'
import {Button} from '@/components/ui/button'
import {DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger} from '@/components/ui/dropdown-menu'
import {cn} from '@/lib/utils'
import type {HierarchyNode, NodeKind} from '@/lib/types'
import {
    addChild,
    childrenOf,
    KIND_COLOR,
    KIND_HINT,
    KIND_LABEL,
    makeNode,
    mapNode,
    moveNode,
    nodeDisplayName,
} from './hierarchyUtils'

const KINDS: NodeKind[] = ['mirror', 'query', 'static', 'drop']

/** "Add a directory of kind …" menu around `trigger`. */
function AddNodeMenu({trigger, onAdd}: { trigger: ReactNode; onAdd: (node: HierarchyNode) => void }) {
    return (
        <DropdownMenu>
            <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-64">
                {KINDS.map((kind) => (
                    <DropdownMenuItem key={kind} onClick={() => onAdd(makeNode(kind))} className="flex-col items-start gap-0.5">
                        <span className="font-medium">{KIND_LABEL[kind]}</span>
                        <span className="text-xs text-muted-foreground">{KIND_HINT[kind]}</span>
                    </DropdownMenuItem>
                ))}
            </DropdownMenuContent>
        </DropdownMenu>
    )
}

interface TreeOps {
    root: HierarchyNode[]
    selectedId: string | null
    onSelect: (id: string) => void
    onChange: (nodes: HierarchyNode[]) => void
}

function NodeRow({node, index, count, depth, ops}: {
    node: HierarchyNode
    index: number
    count: number
    depth: number
    ops: TreeOps
}) {
    const kids = childrenOf(node)
    const add = (child: HierarchyNode) => {
        ops.onChange(addChild(ops.root, node.id, child))
        ops.onSelect(child.id)
    }
    const icon = 'h-6 w-6 text-muted-foreground'
    return (
        <div>
            <div
                role="button"
                tabIndex={0}
                onClick={() => ops.onSelect(node.id)}
                onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && ops.onSelect(node.id)}
                className={cn(
                    'group flex cursor-pointer items-center gap-1.5 rounded-md py-1 pr-1 text-sm',
                    ops.selectedId === node.id ? 'bg-primary/10 text-primary' : 'hover:bg-muted',
                )}
                style={{paddingLeft: depth * 14 + 6}}
            >
                <Badge variant="secondary" className={cn('h-4 shrink-0 border-0 px-1 text-[10px] font-medium', KIND_COLOR[node.kind])}>
                    {KIND_LABEL[node.kind]}
                </Badge>
                <span className="min-w-0 flex-1 truncate">{nodeDisplayName(node)}</span>
                <span className="flex shrink-0 opacity-0 group-hover:opacity-100" onClick={(e) => e.stopPropagation()}>
                    <Button variant="ghost" size="icon" className={icon} disabled={index === 0}
                            onClick={() => ops.onChange(moveNode(ops.root, node.id, -1))} aria-label="Move up">
                        <ChevronUp className="h-3.5 w-3.5"/>
                    </Button>
                    <Button variant="ghost" size="icon" className={icon} disabled={index === count - 1}
                            onClick={() => ops.onChange(moveNode(ops.root, node.id, 1))} aria-label="Move down">
                        <ChevronDown className="h-3.5 w-3.5"/>
                    </Button>
                    {kids && (
                        <AddNodeMenu
                            onAdd={add}
                            trigger={
                                <Button variant="ghost" size="icon" className={icon} aria-label="Add sub-folder">
                                    <Plus className="h-3.5 w-3.5"/>
                                </Button>
                            }
                        />
                    )}
                    <Button variant="ghost" size="icon" className={cn(icon, 'hover:text-destructive')}
                            onClick={() => ops.onChange(mapNode(ops.root, node.id, () => null))} aria-label="Delete node">
                        <Trash2 className="h-3.5 w-3.5"/>
                    </Button>
                </span>
            </div>
            {kids?.map((child, i) => (
                <NodeRow key={child.id} node={child} index={i} count={kids.length} depth={depth + 1} ops={ops}/>
            ))}
        </div>
    )
}

/** The authored node tree — the master half of the hierarchy editor (feature 39 §9). */
export function NodeTree({nodes, selectedId, onSelect, onChange}: {
    nodes: HierarchyNode[]
    selectedId: string | null
    onSelect: (id: string) => void
    onChange: (nodes: HierarchyNode[]) => void
}) {
    const ops: TreeOps = {root: nodes, selectedId, onSelect, onChange}
    return (
        <div className="space-y-2">
            <div>
                {nodes.map((node, i) => (
                    <NodeRow key={node.id} node={node} index={i} count={nodes.length} depth={0} ops={ops}/>
                ))}
                {nodes.length === 0 && <p className="px-2 py-3 text-xs text-muted-foreground">No directories yet.</p>}
            </div>
            <AddNodeMenu
                onAdd={(node) => {
                    onChange(addChild(nodes, null, node))
                    onSelect(node.id)
                }}
                trigger={
                    <Button variant="outline" size="sm" className="w-full gap-1.5">
                        <FolderPlus className="h-4 w-4"/>
                        Add directory
                    </Button>
                }
            />
        </div>
    )
}
