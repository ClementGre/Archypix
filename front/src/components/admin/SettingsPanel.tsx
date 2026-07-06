import {useState} from 'react'
import {Check, ChevronDown, ChevronRight, Lock, RotateCcw, X} from 'lucide-react'
import {toast} from 'sonner'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {NumberInput} from '@/components/ui/number-input'
import {Switch} from '@/components/ui/switch'
import {Badge} from '@/components/ui/badge'
import {Select, SelectContent, SelectItem, SelectTrigger, SelectValue} from '@/components/ui/select'
import {FieldInfoPopover} from '@/components/admin/FieldInfoPopover'
import {cn} from '@/lib/utils'
import {apiErrorMessage} from '@/api/client'
import type {FieldMeta} from '@/lib/types'

// ── value ⇄ input-string conversion ──────────────────────────────────────────────

function valueToText(v: unknown): string {
    if (v === null || v === undefined) return ''
    if (Array.isArray(v)) return (v as unknown[]).join(', ')
    if (typeof v === 'object') return JSON.stringify(v)
    return String(v)
}

function textToValue(kind: string, text: string): unknown {
    const t = text.trim()
    switch (kind) {
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

/** Per-kind numeric constraints so the input can't submit an out-of-range value (feature 24). */
function numericProps(kind: string): { min?: number; max?: number; step: number | 'any' } | null {
    switch (kind) {
        case 'u16':
            return {min: 0, max: 65535, step: 1}
        case 'u64':
        case 'usize':
            return {min: 0, step: 1}
        case 'i64':
            return {step: 1}
        case 'f64':
            return {min: 0, step: 'any'}
        default:
            return null
    }
}

function Tags({field}: { field: FieldMeta }) {
    return (
        <>
            {!field.runtime_editable && (
                <Badge variant="secondary" className="h-5 bg-red-500/15 px-1.5 text-[10px] text-red-500">core</Badge>
            )}
            {field.secret && (
                <Badge variant="secondary" className="h-5 bg-muted px-1.5 text-[10px] text-muted-foreground">secret</Badge>
            )}
            {field.locked && (
                <span className="inline-flex items-center gap-0.5 text-[10px] text-amber-600 dark:text-amber-500">
                    <Lock className="h-3 w-3"/> env
                </span>
            )}
        </>
    )
}

function Provenance({source}: { source: FieldMeta['source'] }) {
    const styles: Record<FieldMeta['source'], string> = {
        default: 'text-muted-foreground',
        env: 'bg-amber-500/15 text-amber-600 dark:text-amber-500',
        db: 'bg-primary/15 text-primary',
    }
    const label = source === 'db' ? 'custom' : source === 'env' ? 'environment' : 'default'
    return <Badge variant="secondary" className={cn('h-5 px-1.5 text-[10px] font-normal', styles[source])}>{label}</Badge>
}

// ── editable field row ────────────────────────────────────────────────────────────

function FieldRow({field, onPatch, onReset}: {
    field: FieldMeta
    onPatch: (key: string, value: unknown) => Promise<void>
    onReset: (key: string) => Promise<void>
}) {
    const [draft, setDraft] = useState<string>(() => valueToText(field.value))
    const [busy, setBusy] = useState(false)
    const serverText = valueToText(field.value)
    const dirty = draft !== serverText
    const locked = field.locked

    const commit = async (value: unknown) => {
        setBusy(true)
        try {
            await onPatch(field.key, value)
            toast.success(`${field.key} saved`)
        } catch (e) {
            toast.error('Could not save setting', {description: apiErrorMessage(e)})
            setDraft(serverText)
        } finally {
            setBusy(false)
        }
    }

    const reset = async () => {
        setBusy(true)
        try {
            await onReset(field.key)
            toast.success(`${field.key} reset to default`)
        } catch (e) {
            toast.error('Could not reset setting', {description: apiErrorMessage(e)})
        } finally {
            setBusy(false)
        }
    }

    const num = numericProps(field.kind)

    let control: React.ReactNode
    if (field.kind === 'bool') {
        control = <Switch checked={field.value === true} disabled={locked || busy} onCheckedChange={(c) => commit(c)}/>
    } else if (field.kind === 'enum') {
        control = (
            <Select value={typeof field.value === 'string' ? field.value : undefined} disabled={locked || busy} onValueChange={(v) => commit(v)}>
                <SelectTrigger className="h-8 w-52"><SelectValue placeholder="—"/></SelectTrigger>
                <SelectContent>{(field.variants ?? []).map((v) => <SelectItem key={v} value={v}>{v}</SelectItem>)}</SelectContent>
            </Select>
        )
    } else {
        const inputProps = {
            value: draft,
            disabled: locked || busy,
            placeholder: field.nullable ? '(unset)' : field.example,
            onChange: (e: React.ChangeEvent<HTMLInputElement>) => setDraft(e.target.value),
            onKeyDown: (e: React.KeyboardEvent<HTMLInputElement>) => {
                if (e.key === 'Enter' && dirty) void commit(textToValue(field.kind, draft))
                if (e.key === 'Escape') setDraft(serverText)
            },
            className: 'h-8 w-52 font-mono text-xs',
        }
        control = (
            <div className="flex items-center gap-1">
                {num ? (
                    <NumberInput {...inputProps} min={num.min} max={num.max} step={num.step}/>
                ) : (
                    <Input {...inputProps} type={field.secret ? 'password' : 'text'}/>
                )}
                {dirty && !locked && (
                    <>
                        <Button size="icon" variant="ghost" className="h-7 w-7 text-emerald-600" disabled={busy}
                                onClick={() => void commit(textToValue(field.kind, draft))} title="Save"><Check className="h-4 w-4"/></Button>
                        <Button size="icon" variant="ghost" className="h-7 w-7 text-muted-foreground" disabled={busy}
                                onClick={() => setDraft(serverText)} title="Discard"><X className="h-4 w-4"/></Button>
                    </>
                )}
            </div>
        )
    }

    return (
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 py-2 last:border-0">
            <div className="flex min-w-0 items-center gap-1.5">
                <span className="font-mono text-sm">{field.key}</span>
                <Provenance source={field.source}/>
                {field.restart_required && <span className="text-[10px] text-muted-foreground">restart</span>}
                <FieldInfoPopover field={field}/>
            </div>
            <div className="flex items-center gap-1.5">
                {control}
                {field.source === 'db' && !locked && (
                    <Button size="icon" variant="ghost" className="h-7 w-7 text-muted-foreground" disabled={busy} onClick={reset}
                            title="Reset to default">
                        <RotateCcw className="h-3.5 w-3.5"/>
                    </Button>
                )}
            </div>
        </div>
    )
}

// ── read-only core field row ──────────────────────────────────────────────────────

function CoreRow({field}: { field: FieldMeta }) {
    return (
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 py-2 last:border-0">
            <div className="flex min-w-0 items-center gap-1.5">
                <span className="font-mono text-sm text-muted-foreground">{field.key}</span>
                <Tags field={field}/>
                <FieldInfoPopover field={field}/>
            </div>
            <span className="max-w-[50%] truncate font-mono text-xs text-muted-foreground">
                {field.secret ? (field.is_set ? '••••••' : '(unset)') : valueToText(field.value) || '(unset)'}
            </span>
        </div>
    )
}

// ── the panel ───────────────────────────────────────────────────────────────────

export interface SettingsPanelProps {
    fields: FieldMeta[]
    onPatch: (key: string, value: unknown) => Promise<void>
    onReset: (key: string) => Promise<void>
    /** When true, omit group headers + the core section (e.g. a single routine's fields). */
    flat?: boolean
}

export function SettingsPanel({fields, onPatch, onReset, flat}: SettingsPanelProps) {
    const [coreOpen, setCoreOpen] = useState(false)
    const [query, setQuery] = useState('')

    if (flat) {
        const editableFlat = fields.filter((f) => f.runtime_editable)
        return editableFlat.length === 0
            ? <p className="text-sm text-muted-foreground">No tunable settings.</p>
            : <div>{editableFlat.map((f) => <FieldRow key={f.key} field={f} onPatch={onPatch} onReset={onReset}/>)}</div>
    }

    const q = query.trim().toLowerCase()
    const match = (f: FieldMeta) =>
        !q || f.key.includes(q) || f.env.toLowerCase().includes(q) || f.description.toLowerCase().includes(q) || f.group.toLowerCase().includes(q)
    const editable = fields.filter((f) => f.runtime_editable && match(f))
    const core = fields.filter((f) => !f.runtime_editable && match(f))

    const groups = new Map<string, FieldMeta[]>()
    for (const f of editable) {
        const g = groups.get(f.group) ?? []
        g.push(f)
        groups.set(f.group, g)
    }

    return (
        <div className="space-y-6">
            <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter settings…"
                className="h-8 max-w-xs"
            />
            {editable.length === 0 && core.length === 0 && (
                <p className="text-sm text-muted-foreground">No settings match “{query}”.</p>
            )}
            {[...groups.entries()].map(([group, groupFields]) => (
                <div key={group}>
                    <h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{group}</h3>
                    <div className="rounded-lg border border-border px-4">
                        {groupFields.map((f) => <FieldRow key={f.key} field={f} onPatch={onPatch} onReset={onReset}/>)}
                    </div>
                </div>
            ))}

            {core.length > 0 && (
                <div>
                    <button className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
                            onClick={() => setCoreOpen((o) => !o)}>
                        {coreOpen ? <ChevronDown className="h-3.5 w-3.5"/> : <ChevronRight className="h-3.5 w-3.5"/>}
                        Core (env-only, read-only) · {core.length}
                    </button>
                    {coreOpen && (
                        <div className="mt-1 rounded-lg border border-border px-4">
                            {core.map((f) => <CoreRow key={f.key} field={f}/>)}
                        </div>
                    )}
                </div>
            )}
        </div>
    )
}
