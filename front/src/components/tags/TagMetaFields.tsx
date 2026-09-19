// The `tag_metadata` form (feature 34 §3, §6), shared by `EditTagDialog` and the *Customize* pane of
// `NewTagDialog` — so a tag can be configured at creation time instead of created then edited.

import type {ReactNode} from 'react'
import {AlertTriangle, Calendar as CalendarIcon, Check, RotateCcw} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Input} from '@/components/ui/input'
import {Label} from '@/components/ui/label'
import {Switch} from '@/components/ui/switch'
import {Textarea} from '@/components/ui/textarea'
import {CoverPicker} from '@/components/tags/CoverPicker'
import {DateTimePickerPopover, formatNaive} from '@/components/photos/detail/DateTimePickerPopover'
import {cn, TagPath} from '@/lib/utils'
import type {TagMeta, TagMetaPatch} from '@/lib/types'

/** A small fixed palette; the colour input beside it stores free hex (§3.2). */
const PALETTE = [
    '#ef4444', '#f97316', '#eab308', '#22c55e',
    '#14b8a6', '#3b82f6', '#8b5cf6', '#ec4899',
]

/** The form's local state — decorative fields only; ordering and view prefs are written elsewhere. */
export interface TagMetaDraft {
    name: string
    description: string
    color: string
    dateFrom: string | null
    dateTo: string | null
    showWhenEmpty: boolean
    webdavDirName: string
    cover: string | null
}

/** The API speaks naive timestamps; the picker wants `YYYY-MM-DDTHH:MM:SS`. */
export function toNaive(iso: string | null | undefined): string | null {
    return iso ? `${iso.replace(' ', 'T').slice(0, 19)}` : null
}

export function draftFromMeta(meta: TagMeta | null | undefined, showWhenEmpty = false): TagMetaDraft {
    return {
        name: meta?.display_name ?? '',
        description: meta?.description ?? '',
        color: meta?.color ?? '',
        dateFrom: toNaive(meta?.date_from),
        dateTo: toNaive(meta?.date_to),
        showWhenEmpty: meta?.show_when_empty ?? showWhenEmpty,
        webdavDirName: meta?.webdav_dir_name ?? '',
        cover: meta?.cover_picture_id ?? null,
    }
}

export function draftToPatch(tagPath: string, draft: TagMetaDraft): TagMetaPatch {
    const name = draft.name.trim()
    return {
        tag_path: tagPath,
        // A name identical to the ltree label is not an override — that is already the fallback (§5).
        display_name: name && name !== TagPath.leaf(tagPath) ? name : null,
        description: draft.description.trim() || null,
        color: draft.color || null,
        date_from: draft.dateFrom,
        date_to: draft.dateTo,
        show_when_empty: draft.showWhenEmpty,
        cover_picture_id: draft.cover,
        webdav_dir_name: draft.webdavDirName.trim() || null,
    }
}

/** The display name, which every other surface falls back from — hence its own field, shown before
 *  the *Customize* fold in the create dialog. */
export function TagNameField({value, onChange, placeholder, onEnter}: {
    value: string
    onChange: (value: string) => void
    placeholder: string
    onEnter?: () => void
}) {
    return (
        <div className="space-y-1.5">
            <Label htmlFor="tag-display-name">Display name</Label>
            <Input
                id="tag-display-name"
                value={value}
                onChange={(e) => onChange(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && onEnter?.()}
                placeholder={placeholder}
                maxLength={128}
            />
            <p className="text-[11px] text-muted-foreground">
                Emoji are welcome — the name travels everywhere the tag does.
            </p>
        </div>
    )
}

/**
 * The tag path as a live-validated display-form field, carrying the validation the `TagPicker`
 * popup used to apply to its own input: what will be auto-fixed in amber, and anything that cannot
 * be mapped in red, so an unusable path is caught before Create rather than silently slugified.
 */
export function TagPathField({value, onChange, children}: {
    value: string
    onChange: (value: string) => void
    /** The hint or reset row under the field. */
    children?: ReactNode
}) {
    const {replaced} = TagPath.sanitize(value)
    const invalid = TagPath.invalidChars(value)

    return (
        <div className="space-y-1.5">
            <Label htmlFor="tag-path">Tag path</Label>
            <Input
                id="tag-path"
                value={value}
                onChange={(e) => onChange(e.target.value)}
                placeholder="/Era/2026/Vietnam"
                className="font-mono text-xs"
            />
            {replaced.length > 0 && (
                <p className="flex items-start gap-1 text-[11px] text-amber-500">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                    <span>{replaced.join(' · ')}</span>
                </p>
            )}
            {invalid.length > 0 && (
                <p className="flex items-start gap-1 text-[11px] text-destructive">
                    <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0"/>
                    <span>
                        Not allowed: {invalid.map((c) => `“${c}”`).join(' ')}. Use letters, numbers, “_” or “/”.
                    </span>
                </p>
            )}
            {children}
        </div>
    )
}

/** One side of the date range: the custom picker, with the derived value as the reset target (§4). */
function DateOverrideField({label, value, derived, onChange}: {
    label: string
    value: string | null
    derived: string | null
    onChange: (value: string | null) => void
}) {
    return (
        <div className="space-y-1.5">
            <Label>{label}</Label>
            <DateTimePickerPopover value={value} onChange={onChange}>
                <Button variant="outline" className="w-full justify-start font-normal">
                    <CalendarIcon className="mr-2 h-3.5 w-3.5 shrink-0 text-muted-foreground"/>
                    <span className={cn('truncate', !value && 'text-muted-foreground')}>
                        {value ? formatNaive(value) : formatNaive(derived) || 'Not set'}
                    </span>
                </Button>
            </DateTimePickerPopover>
            <button
                type="button"
                onClick={() => onChange(null)}
                className="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
            >
                <RotateCcw className="h-3 w-3"/>
                Derived: {formatNaive(derived) || '—'}
            </button>
        </div>
    )
}

/** Everything but the display name: description, colour, cover, date overrides, empty visibility and
 *  the WebDAV folder name. */
export function TagMetaFields({draft, onChange, label, derived}: {
    draft: TagMetaDraft
    onChange: (patch: Partial<TagMetaDraft>) => void
    /** The ltree leaf label a blank name or folder name falls back to. */
    label: string
    /** Derived MIN/MAX(captured_at); a tag with no photos yet has neither. */
    derived?: { from: string | null; to: string | null }
}) {
    return (
        <>
            <div className="space-y-1.5">
                <Label htmlFor="tag-description">Description</Label>
                <Textarea
                    id="tag-description"
                    value={draft.description}
                    onChange={(e) => onChange({description: e.target.value})}
                    rows={2}
                    maxLength={2000}
                />
            </div>

            <div className="space-y-1.5">
                <Label>Colour</Label>
                <div className="flex flex-wrap items-center gap-1.5">
                    {PALETTE.map((c) => (
                        <button
                            key={c}
                            type="button"
                            onClick={() => onChange({color: draft.color === c ? '' : c})}
                            style={{backgroundColor: c}}
                            className={cn(
                                'flex h-6 w-6 items-center justify-center rounded-full border',
                                draft.color === c ? 'ring-2 ring-offset-1 ring-ring' : 'border-transparent',
                            )}
                            aria-label={`Colour ${c}`}
                        >
                            {draft.color === c && <Check className="h-3 w-3 text-white"/>}
                        </button>
                    ))}
                    <Input
                        type="color"
                        value={draft.color || '#888888'}
                        onChange={(e) => onChange({color: e.target.value})}
                        className="h-6 w-10 cursor-pointer p-0.5"
                        aria-label="Custom colour"
                    />
                    {draft.color && (
                        <Button variant="ghost" size="sm" className="h-6 px-2 text-xs"
                                onClick={() => onChange({color: ''})}>
                            Clear
                        </Button>
                    )}
                </div>
            </div>

            <CoverPicker value={draft.cover} onChange={(cover) => onChange({cover})}/>

            {/* An unset side shows the derived value; resetting a side clears the override (§4). */}
            <div className="grid grid-cols-2 gap-3">
                <DateOverrideField label="From" value={draft.dateFrom} derived={derived?.from ?? null}
                                   onChange={(dateFrom) => onChange({dateFrom})}/>
                <DateOverrideField label="To" value={draft.dateTo} derived={derived?.to ?? null}
                                   onChange={(dateTo) => onChange({dateTo})}/>
            </div>

            <div className="flex items-center justify-between gap-3">
                <div>
                    <Label htmlFor="tag-show-empty">Show when empty</Label>
                    <p className="text-[11px] text-muted-foreground">
                        Keep the tag in the tree even with no photos.
                    </p>
                </div>
                <Switch id="tag-show-empty" checked={draft.showWhenEmpty}
                        onCheckedChange={(showWhenEmpty) => onChange({showWhenEmpty})}/>
            </div>

            <div className="space-y-1.5">
                <Label htmlFor="tag-webdav">WebDAV folder name</Label>
                <div className="flex items-center gap-2">
                    <Input
                        id="tag-webdav"
                        value={draft.webdavDirName}
                        onChange={(e) => onChange({webdavDirName: e.target.value})}
                        placeholder={label}
                        maxLength={255}
                    />
                    <Button
                        variant="outline"
                        size="sm"
                        disabled={!draft.name.trim()}
                        onClick={() => onChange({webdavDirName: draft.name.trim()})}
                    >
                        Use display name
                    </Button>
                </div>
                <p className="text-[11px] text-muted-foreground">
                    Set once, deliberately — a mounted client sees a folder rename as delete + create, so
                    this never follows the display name on its own.
                </p>
            </div>
        </>
    )
}
