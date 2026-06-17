import {ChevronLeft, FolderTree, Loader2, Pencil, Plus} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {useHierarchies} from '@/hooks/useHierarchies'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import {HierarchyDirTree} from './HierarchyDirTree'
import {CreateHierarchyDialog} from './CreateHierarchyDialog'

/** Hierarchies tab: a list of hierarchies, and the directory tree of the active one. */
export function HierarchyPanel() {
    const {data: hierarchies, isPending, isError, error} = useHierarchies()
    const {params, update} = useGalleryParams()

    const active = hierarchies?.find((h) => h.id === params.hierarchy) ?? null

    // ── Active hierarchy: show its directory tree ──────────────────────────────
    if (active) {
        return (
            <div className="flex h-full flex-col">
                <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0 text-muted-foreground"
                        onClick={() => update({hierarchy: null, hpath: '', hedit: null})}
                        aria-label="Back to hierarchies"
                        title="All hierarchies"
                    >
                        <ChevronLeft className="h-4 w-4"/>
                    </Button>
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">{active.name}</span>
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0 text-muted-foreground"
                        onClick={() => update({hedit: active.id})}
                        aria-label="Edit hierarchy"
                        title="Edit hierarchy"
                    >
                        <Pencil className="h-3.5 w-3.5"/>
                    </Button>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto">
                    <HierarchyDirTree hierarchyId={active.id}/>
                </div>
            </div>
        )
    }

    // ── No active hierarchy: list ──────────────────────────────────────────────
    return (
        <div className="flex h-full flex-col">
            <div className="flex items-center justify-between border-b border-border px-3 py-2">
                <span className="text-sm font-medium">Hierarchies</span>
                <CreateHierarchyDialog
                    onCreated={(id) => update({hierarchy: id, hpath: '', hedit: id, panel: 'hierarchies'})}
                    trigger={
                        <Button variant="ghost" size="sm" className="h-7 gap-1.5 text-xs">
                            <Plus className="h-3.5 w-3.5"/>
                            New
                        </Button>
                    }
                />
            </div>

            <div className="min-h-0 flex-1 overflow-y-auto p-2">
                {isPending && (
                    <div className="flex items-center justify-center py-6 text-muted-foreground">
                        <Loader2 className="h-4 w-4 animate-spin"/>
                    </div>
                )}
                {isError && <p className="px-2 py-4 text-xs text-muted-foreground">{apiErrorMessage(error)}</p>}
                {!isPending && !isError && hierarchies && hierarchies.length === 0 && (
                    <div className="flex flex-col items-center gap-2 px-4 py-10 text-center text-xs text-muted-foreground">
                        <FolderTree className="h-6 w-6"/>
                        <p className="font-medium text-foreground">No hierarchies yet</p>
                        <p>Create one to browse your tags as a folder tree.</p>
                    </div>
                )}
                {hierarchies?.map((h) => (
                    <div
                        key={h.id}
                        className="group flex items-center gap-1 rounded-md px-1 text-sm hover:bg-muted"
                    >
                        <button
                            onClick={() => update({hierarchy: h.id, hpath: '', hedit: null})}
                            className="flex min-w-0 flex-1 items-center gap-2 py-1.5 pl-1.5"
                        >
                            <FolderTree className={cn('h-4 w-4 shrink-0', !h.enabled && 'opacity-40')}/>
                            <span className={cn('truncate', !h.enabled && 'text-muted-foreground')}>{h.name}</span>
                            {!h.enabled && (
                                <span className="shrink-0 text-[11px] uppercase tracking-wide text-muted-foreground">
                                    off
                                </span>
                            )}
                        </button>
                        <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100"
                            onClick={() => update({hedit: h.id, hierarchy: h.id})}
                            aria-label={`Edit ${h.name}`}
                            title="Edit"
                        >
                            <Pencil className="h-3.5 w-3.5"/>
                        </Button>
                    </div>
                ))}
            </div>
        </div>
    )
}
