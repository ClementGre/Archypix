import {useState} from 'react'
import {Check, Copy, Info} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import type {FieldMeta} from '@/lib/types'

function valueToText(v: unknown): string {
    if (v === null || v === undefined) return ''
    if (Array.isArray(v)) return (v as unknown[]).join(', ')
    if (typeof v === 'object') return JSON.stringify(v)
    return String(v)
}

/** Human-friendly type label instead of the raw Rust kind (`u64` → "unsigned integer"). */
function friendlyKind(kind: string): string {
    switch (kind) {
        case 'u16':
            return 'unsigned integer (0–65535)'
        case 'u64':
        case 'usize':
            return 'unsigned integer'
        case 'i64':
            return 'integer'
        case 'f64':
            return 'number'
        case 'bool':
            return 'boolean'
        case 'string':
            return 'text'
        case 'string_list':
            return 'list of text'
        case 'enum':
            return 'choice'
        default:
            return kind
    }
}

function CopyInline({text}: { text: string }) {
    const [copied, setCopied] = useState(false)
    return (
        <button
            className="inline-flex items-center gap-1 break-all rounded bg-muted px-1.5 py-0.5 text-left font-mono text-[11px] hover:bg-muted/70"
            onClick={async () => {
                try {
                    await navigator.clipboard.writeText(text)
                    setCopied(true)
                    setTimeout(() => setCopied(false), 1200)
                } catch {
                    /* clipboard unavailable */
                }
            }}
        >
            {text}
            {copied ? <Check className="h-3 w-3 text-emerald-500"/> : <Copy className="h-3 w-3 text-muted-foreground"/>}
        </button>
    )
}

/** Hover/click info for a setting: description + copyable env name, type, default, example. */
export function FieldInfoPopover({field}: { field: FieldMeta }) {
    return (
        <Popover>
            <PopoverTrigger asChild>
                <Button variant="ghost" size="icon" className="h-6 w-6 text-muted-foreground" aria-label="Field info">
                    <Info className="h-3.5 w-3.5"/>
                </Button>
            </PopoverTrigger>
            <PopoverContent className="w-80 space-y-2 text-xs" align="end">
                {field.description && <p>{field.description}</p>}
                <dl className="space-y-1">
                    <div className="flex items-start justify-between gap-2">
                        {/* Non-breaking space keeps the label on one line so the (long) env value wraps instead. */}
                        <dt className="shrink-0 text-muted-foreground">Env&nbsp;var</dt>
                        <dd className="min-w-0 text-right"><CopyInline text={field.env}/></dd>
                    </div>
                    <div className="flex items-center justify-between gap-2">
                        <dt className="text-muted-foreground">Type</dt>
                        <dd>{friendlyKind(field.kind)}{field.nullable ? ' (optional)' : ''}</dd>
                    </div>
                    {field.default_value !== null && field.default_value !== undefined && (
                        <div className="flex items-center justify-between gap-2">
                            <dt className="text-muted-foreground">Default</dt>
                            <dd className="font-mono">{valueToText(field.default_value) || '—'}</dd>
                        </div>
                    )}
                    {/* Only show the example when it adds information (i.e. differs from the default). */}
                    {field.example && valueToText(field.example) !== valueToText(field.default_value) && (
                        <div className="flex items-center justify-between gap-2">
                            <dt className="text-muted-foreground">Example</dt>
                            <dd className="font-mono">{field.example}</dd>
                        </div>
                    )}
                    {field.restart_required && (
                        <p className="text-amber-600 dark:text-amber-500">Takes effect after a restart.</p>
                    )}
                </dl>
            </PopoverContent>
        </Popover>
    )
}
