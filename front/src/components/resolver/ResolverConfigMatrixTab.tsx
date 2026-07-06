import {Fragment, useMemo, useState} from 'react'
import {AlertTriangle, Check, Loader2} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Skeleton} from '@/components/ui/skeleton'
import {Badge} from '@/components/ui/badge'
import {FieldInfoPopover} from '@/components/admin/FieldInfoPopover'
import {useConfigMatrix, useConfigMatrixPatch} from '@/hooks/useResolverAdmin'
import {apiErrorMessage} from '@/api/client'
import {cn} from '@/lib/utils'
import type {ConfigMatrixResponse} from '@/api/resolverAdmin'
import type {FieldMeta} from '@/lib/types'

function valueText(v: unknown): string {
    if (v === null || v === undefined) return '—'
    if (Array.isArray(v)) return v.join(', ')
    if (typeof v === 'object') return JSON.stringify(v)
    return String(v)
}

function textToValue(kind: string, text: string): unknown {
    const t = text.trim()
    switch (kind) {
        case 'bool':
            return t === 'true' || t === '1'
        case 'string_list':
            return t ? t.split(',').map((s) => s.trim()).filter(Boolean) : []
        case 'u16':
        case 'u64':
        case 'usize':
        case 'i64':
        case 'f64':
            return t === '' ? null : Number(t)
        default:
            return t
    }
}

interface Row {
    meta: FieldMeta
    values: Record<string, string | undefined>
    diverges: boolean
}

function SetAllControl({row}: { row: Row }) {
    const [draft, setDraft] = useState('')
    const patch = useConfigMatrixPatch()

    const apply = async () => {
        try {
            const results = await patch.mutateAsync({key: row.meta.key, value: textToValue(row.meta.kind, draft)})
            const failures = Object.entries(results).filter(([, r]) => !r.ok)
            if (failures.length === 0) {
                toast.success(`${row.meta.key} set on all backends`)
                setDraft('')
            } else {
                toast.warning(`${row.meta.key}: ${failures.length} backend(s) rejected it`, {
                    description: failures.map(([d, r]) => `${d}: ${r.error ?? r.status}`).join('; '),
                })
            }
        } catch (e) {
            toast.error('Fan-out failed', {description: apiErrorMessage(e)})
        }
    }

    return (
        <div className="flex items-center gap-1">
            <Input
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                placeholder={row.meta.kind === 'bool' ? 'true / false' : 'set all…'}
                onKeyDown={(e) => {
                    if (e.key === 'Enter' && draft.trim()) void apply()
                }}
                className="h-7 w-40 font-mono text-xs"
            />
            <Button size="icon" variant="ghost" className="h-6 w-6 text-emerald-600" disabled={!draft.trim() || patch.isPending}
                    onClick={apply} title="Apply to all reachable backends">
                {patch.isPending ? <Loader2 className="h-3.5 w-3.5 animate-spin"/> : <Check className="h-3.5 w-3.5"/>}
            </Button>
        </div>
    )
}

export function ResolverConfigMatrixTab() {
    const {data, isLoading} = useConfigMatrix()
    const {backends, groups, errors} = useMemo(() => buildMatrix(data), [data])

    if (isLoading) {
        return <div className="space-y-3">{Array.from({length: 6}).map((_, i) => <Skeleton key={i} className="h-8 w-full"/>)}</div>
    }
    if (backends.length === 0) {
        return <p className="text-sm text-muted-foreground">No reachable backends to compare.</p>
    }

    const colCount = backends.length + 2

    return (
        <div className="space-y-3">
            <p className="text-sm text-muted-foreground">
                Runtime settings across every reachable backend. Diverging fields are highlighted; “set all”
                fans a value out to the fleet (best-effort — locked/failed backends are reported per field).
            </p>
            {errors.length > 0 && (
                <p className="flex items-center gap-2 rounded-md bg-amber-500/10 px-3 py-2 text-xs text-amber-600 dark:text-amber-500">
                    <AlertTriangle className="h-4 w-4"/>
                    {errors.map(([d, e]) => `${d}: ${e}`).join(' · ')}
                </p>
            )}
            <div className="overflow-x-auto rounded-lg border border-border">
                <table className="w-full min-w-[640px] text-sm">
                    <thead>
                    <tr className="border-b border-border bg-muted/40 text-left text-xs text-muted-foreground">
                        <th className="p-2 font-medium">Field</th>
                        {backends.map((b) => <th key={b} className="p-2 font-mono font-medium">{b}</th>)}
                        <th className="p-2 font-medium">Set all</th>
                    </tr>
                    </thead>
                    <tbody>
                    {[...groups.entries()].map(([group, rows]) => (
                        <Fragment key={group}>
                            <tr className="bg-muted/20">
                                <td colSpan={colCount} className="px-2 py-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                                    {group}
                                </td>
                            </tr>
                            {rows.map((row) => (
                                <tr key={row.meta.key} className="border-b border-border/60 align-top">
                                    <td className="p-2">
                                        <div className="flex items-center gap-1">
                                            <span className="font-mono text-xs">{row.meta.key}</span>
                                            <FieldInfoPopover field={row.meta}/>
                                            {row.diverges && <Badge variant="secondary"
                                                                    className="h-4 bg-amber-500/15 px-1 text-[9px] text-amber-600 dark:text-amber-500">diverges</Badge>}
                                        </div>
                                    </td>
                                    {backends.map((b) => {
                                        const v = row.values[b]
                                        return (
                                            <td key={b} className={cn('p-2 font-mono text-xs', row.diverges && 'bg-amber-500/5')}>
                                                {v === undefined ? <span className="italic text-muted-foreground/60">n/a</span> : v}
                                            </td>
                                        )
                                    })}
                                    <td className="p-2"><SetAllControl row={row}/></td>
                                </tr>
                            ))}
                        </Fragment>
                    ))}
                    </tbody>
                </table>
            </div>
        </div>
    )
}

function buildMatrix(data: ConfigMatrixResponse | undefined) {
    const backends: string[] = []
    const errors: [string, string][] = []
    const perBackendFields: Record<string, FieldMeta[]> = {}

    for (const [domain, payload] of Object.entries(data ?? {})) {
        if (Array.isArray(payload)) {
            backends.push(domain)
            perBackendFields[domain] = payload
        } else {
            errors.push([domain, payload.error])
        }
    }

    // Union of runtime-editable keys, preserving first-seen order + group.
    const keyOrder: string[] = []
    const meta: Record<string, FieldMeta> = {}
    for (const domain of backends) {
        for (const f of perBackendFields[domain]) {
            if (!f.runtime_editable) continue
            if (!(f.key in meta)) {
                meta[f.key] = f
                keyOrder.push(f.key)
            }
        }
    }

    const groups = new Map<string, Row[]>()
    for (const key of keyOrder) {
        const values: Record<string, string | undefined> = {}
        const distinct = new Set<string>()
        for (const domain of backends) {
            const f = perBackendFields[domain].find((x) => x.key === key)
            const text = f ? valueText(f.value) : undefined
            values[domain] = text
            if (text !== undefined) distinct.add(text)
        }
        const row: Row = {meta: meta[key], values, diverges: distinct.size > 1}
        const g = groups.get(meta[key].group) ?? []
        g.push(row)
        groups.set(meta[key].group, g)
    }

    return {backends, groups, errors}
}
