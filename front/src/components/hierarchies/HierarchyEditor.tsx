import {useState} from 'react'
import {toast} from 'sonner'
import {Braces, Loader2, Save, Trash2, Undo2, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {Skeleton} from '@/components/ui/skeleton'
import {ConfirmDialog} from '@/components/common/ConfirmDialog'
import {useHierarchy, useHierarchyMutations} from '@/hooks/useHierarchies'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {apiErrorMessage} from '@/api/client'
import type {HierarchyConfig, NamingStrategy, SafeDeleteMode} from '@/lib/types'
import {NodeListEditor} from './NodeEditor'
import {JsonConfigDialog} from './JsonConfigDialog'
import {NAMING_OPTIONS, SAFE_DELETE_OPTIONS} from './hierarchyUtils'

interface Draft {
    name: string
    enabled: boolean
    config: HierarchyConfig
}

export function HierarchyEditor({id}: { id: string }) {
    const {data, isPending, isError, error} = useHierarchy(id)
    const {update, remove} = useHierarchyMutations()
    const {update: updateParams} = useGalleryParams()

    const [draft, setDraft] = useState<Draft | null>(null)
    const [sig, setSig] = useState('')

    // Reseed the draft whenever the server record changes (id or updated_at).
    if (data) {
        const newSig = `${data.id}:${data.updated_at}`
        if (newSig !== sig) {
            setSig(newSig)
            setDraft({name: data.name, enabled: data.enabled, config: data.config})
        }
    }

    const close = () => updateParams({hedit: null})

    const serverSnapshot = data ? JSON.stringify({name: data.name, enabled: data.enabled, config: data.config}) : ''
    const dirty = !!draft && JSON.stringify(draft) !== serverSnapshot

    const setConfig = (patch: Partial<HierarchyConfig>) =>
        setDraft((d) => (d ? {...d, config: {...d.config, ...patch}} : d))

    const save = () => {
        if (!draft) return
        update.mutate(
            {id, body: {name: draft.name.trim(), enabled: draft.enabled, config: draft.config}},
            {
                onSuccess: () => toast.success('Hierarchy saved'),
                onError: (e) => toast.error(apiErrorMessage(e)),
            },
        )
    }

    const del = () =>
        remove.mutate(id, {
            onSuccess: () => {
                toast.success('Hierarchy deleted')
                updateParams({hedit: null, hierarchy: null, hpath: ''})
            },
            onError: (e) => toast.error(apiErrorMessage(e)),
        })

    return (
        <div className="flex h-full min-h-0 flex-col">
            {/* Header bar */}
            <div className="flex items-center gap-2 border-b border-border px-3 py-2">
                <Button variant="ghost" size="icon" className="h-8 w-8 shrink-0" onClick={close} aria-label="Close editor">
                    <X className="h-4 w-4"/>
                </Button>
                <span className="text-sm font-medium text-muted-foreground">Edit hierarchy</span>
                <div className="flex-1"/>
                {data && (
                    <>
                        <JsonConfigDialog
                            config={draft?.config ?? data.config}
                            onApply={(config) => setDraft((d) => (d ? {...d, config} : d))}
                            trigger={
                                <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-muted-foreground" title="Edit raw JSON (debug)">
                                    <Braces className="h-4 w-4"/>
                                </Button>
                            }
                        />
                        <ConfirmDialog
                            title="Delete hierarchy?"
                            description="This removes the hierarchy view. Your tags and pictures are untouched."
                            confirmLabel="Delete"
                            destructive
                            onConfirm={del}
                            trigger={
                                <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-muted-foreground hover:text-destructive">
                                    <Trash2 className="h-4 w-4"/>
                                </Button>
                            }
                        />
                        {dirty && (
                            <Button
                                variant="ghost"
                                size="sm"
                                className="h-8 gap-1.5 text-muted-foreground"
                                onClick={() => data && setDraft({name: data.name, enabled: data.enabled, config: data.config})}
                                disabled={update.isPending}
                            >
                                <Undo2 className="h-4 w-4"/>
                                Reset
                            </Button>
                        )}
                        <Button size="sm" className="h-8 gap-1.5" onClick={save} disabled={!dirty || update.isPending}>
                            {update.isPending ? <Loader2 className="h-4 w-4 animate-spin"/> : <Save className="h-4 w-4"/>}
                            Save
                        </Button>
                    </>
                )}
            </div>

            <div className="min-h-0 flex-1 overflow-y-auto p-6">
                <div className="mx-auto max-w-3xl space-y-6">
                    {isPending && (
                        <div className="space-y-4">
                            <Skeleton className="h-9 w-64"/>
                            <Skeleton className="h-32 w-full"/>
                            <Skeleton className="h-40 w-full"/>
                        </div>
                    )}
                    {isError && (
                        <div className="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
                            {apiErrorMessage(error)}
                        </div>
                    )}

                    {draft && data && (
                        <>
                            {/* Identity + settings */}
                            <section className="space-y-4 rounded-lg border p-4">
                                <div className="flex items-end gap-4">
                                    <div className="flex-1 space-y-1">
                                        <Label htmlFor="h-name" className="text-xs text-muted-foreground">Name</Label>
                                        <Input
                                            id="h-name"
                                            value={draft.name}
                                            onChange={(e) => setDraft({...draft, name: e.target.value})}
                                        />
                                    </div>
                                    <label className="flex items-center gap-2 pb-2 text-sm">
                                        <Switch
                                            checked={draft.enabled}
                                            onCheckedChange={(v) => setDraft({...draft, enabled: v})}
                                        />
                                        <span>{draft.enabled ? 'Enabled' : 'Disabled'}</span>
                                    </label>
                                </div>

                                <div className="grid grid-cols-2 gap-4">
                                    <div className="space-y-1">
                                        <Label className="text-xs text-muted-foreground">Default naming</Label>
                                        <Select
                                            value={draft.config.naming}
                                            onValueChange={(v) => setConfig({naming: v as NamingStrategy})}
                                        >
                                            <SelectTrigger className="h-9">
                                                <SelectValue/>
                                            </SelectTrigger>
                                            <SelectContent>
                                                {NAMING_OPTIONS.map((o) => (
                                                    <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                                ))}
                                            </SelectContent>
                                        </Select>
                                    </div>
                                    <div className="space-y-1">
                                        <Label className="text-xs text-muted-foreground">Default safe delete</Label>
                                        <Select
                                            value={draft.config.safeDeleteMode}
                                            onValueChange={(v) => setConfig({safeDeleteMode: v as SafeDeleteMode})}
                                        >
                                            <SelectTrigger className="h-9">
                                                <SelectValue/>
                                            </SelectTrigger>
                                            <SelectContent>
                                                {SAFE_DELETE_OPTIONS.map((o) => (
                                                    <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>
                                                ))}
                                            </SelectContent>
                                        </Select>
                                    </div>
                                </div>

                                <label className="flex items-center gap-2 text-sm">
                                    <Switch
                                        checked={draft.config.writeBack}
                                        onCheckedChange={(v) => setConfig({writeBack: v})}
                                    />
                                    <span>
                                        Allow write-back
                                        <span className="ml-1 text-xs text-muted-foreground">
                                            (master switch; off ⇒ entire hierarchy read-only)
                                        </span>
                                    </span>
                                </label>
                            </section>

                            {/* Node tree */}
                            <section className="space-y-3">
                                <div>
                                    <h2 className="text-sm font-medium">Directories</h2>
                                    <p className="text-xs text-muted-foreground">
                                        Each node renders to a directory. Mirror expands a tag subtree; query filters by a
                                        predicate and may nest; static is a plain container; drop is a write-only inbox.
                                    </p>
                                </div>
                                <NodeListEditor
                                    nodes={draft.config.nodes}
                                    onChange={(nodes) => setConfig({nodes})}
                                    wb={{master: draft.config.writeBack, inherited: true}}
                                />
                            </section>
                        </>
                    )}
                </div>
            </div>
        </div>
    )
}
