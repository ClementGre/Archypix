import {useState} from 'react'
import {ChevronRight, Folder, FolderOpen, Loader2, Lock} from 'lucide-react'
import {useHierarchyTree} from '@/hooks/useHierarchies'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import type {DirEntry} from '@/lib/types'
import {cn} from '@/lib/utils'

/** A single directory row that lazily loads its children when expanded. */
function DirRow({
                    hierarchyId,
                    entry,
                    path,
                    depth,
                }: {
    hierarchyId: string
    entry: DirEntry
    path: string // full path of this directory (slash-separated names)
    depth: number
}) {
    const {params, update} = useGalleryParams()
    const [open, setOpen] = useState(false)
    const hasChildren = entry.child_count > 0
    const isActive = params.hpath === path

    const {data, isPending, isError, error} = useHierarchyTree(hierarchyId, path, {
        enabled: open && hasChildren,
    })

    return (
        <div>
            <div
                onClick={() => update({hierarchy: hierarchyId, hpath: path, hedit: null})}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault()
                        update({hierarchy: hierarchyId, hpath: path, hedit: null})
                    }
                }}
                title={entry.writable ? undefined : 'Read-only directory'}
                className={cn(
                    'group flex cursor-pointer items-center gap-1 rounded-md py-1 pr-2 text-sm',
                    isActive ? 'bg-primary/10 text-primary' : 'text-foreground hover:bg-muted',
                )}
                style={{paddingLeft: depth * 12 + 4}}
            >
                <button
                    onClick={(e) => {
                        e.stopPropagation()
                        if (hasChildren) setOpen((o) => !o)
                    }}
                    className={cn('flex h-4 w-4 shrink-0 items-center justify-center', !hasChildren && 'invisible')}
                    aria-label={open ? 'Collapse' : 'Expand'}
                >
                    <ChevronRight className={cn('h-3.5 w-3.5 transition-transform', open && 'rotate-90')}/>
                </button>
                <div className="flex min-w-0 flex-1 items-center gap-1.5">
                    {open && hasChildren ? (
                        <FolderOpen className="h-3.5 w-3.5 shrink-0 opacity-70"/>
                    ) : (
                        <Folder className="h-3.5 w-3.5 shrink-0 opacity-70"/>
                    )}
                    <span className="truncate">{entry.name}</span>
                    {!entry.writable && <Lock className="h-3 w-3 shrink-0 opacity-40"/>}
                    {entry.picture_count != null && entry.picture_count > 0 && (
                        <span className="ml-auto shrink-0 pl-1 text-[11px] tabular-nums text-muted-foreground">
                            {entry.picture_count}
                        </span>
                    )}
                </div>
            </div>

            {open && hasChildren && (
                <div>
                    {isPending && (
                        <div className="flex items-center py-1 text-muted-foreground" style={{paddingLeft: (depth + 1) * 12 + 8}}>
                            <Loader2 className="h-3.5 w-3.5 animate-spin"/>
                        </div>
                    )}
                    {isError && (
                        <p className="px-3 py-1 text-xs text-muted-foreground" style={{paddingLeft: (depth + 1) * 12 + 8}}>
                            {apiErrorMessage(error)}
                        </p>
                    )}
                    {data?.directories.map((child) => (
                        <DirRow
                            key={child.name}
                            hierarchyId={hierarchyId}
                            entry={child}
                            path={path ? `${path}/${child.name}` : child.name}
                            depth={depth + 1}
                        />
                    ))}
                </div>
            )}
        </div>
    )
}

/** The navigable directory tree of one hierarchy; clicking a folder drives the center grid. */
export function HierarchyDirTree({hierarchyId}: { hierarchyId: string }) {
    const {params, update} = useGalleryParams()
    const {data, isPending, isError, error} = useHierarchyTree(hierarchyId, '')

    return (
        <div className="px-1 py-1">
            <button
                onClick={() => update({hierarchy: hierarchyId, hpath: '', hedit: null})}
                className={cn(
                    'mb-0.5 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium',
                    params.hpath === '' ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:bg-muted',
                )}
            >
                <FolderOpen className="h-4 w-4"/>
                All directories
            </button>

            {isPending && (
                <div className="flex items-center justify-center py-6 text-muted-foreground">
                    <Loader2 className="h-4 w-4 animate-spin"/>
                </div>
            )}
            {isError && <p className="px-3 py-4 text-xs text-muted-foreground">{apiErrorMessage(error)}</p>}
            {!isPending && !isError && data && data.directories.length === 0 && (
                <p className="px-3 py-4 text-xs text-muted-foreground">
                    No directories — this hierarchy has no nodes, or none match any pictures.
                </p>
            )}
            {data?.directories.map((entry) => (
                <DirRow key={entry.name} hierarchyId={hierarchyId} entry={entry} path={entry.name} depth={0}/>
            ))}
        </div>
    )
}
