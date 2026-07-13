import {useState} from 'react'
import {Check, RotateCcw, Share2, User, X} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {ContactInput} from '@/components/common/ContactInput'
import {useSetCreator} from '@/hooks/usePictureEdit'
import {cn} from '@/lib/utils'
import type {PictureDetail} from '@/lib/types'

type ParsedCreator =
    | { kind: 'identity'; handle: string } // @user:domain — a verified Archypix identity
    | { kind: 'anon'; name: string } // #name — an unauthenticated public uploader (feature 27)
    | { kind: 'plain'; text: string } // arbitrary manual credit

/** Parse a creator string by its leading sigil (feature 26 §3) — total, no throw. */
function parseCreator(value: string): ParsedCreator {
    if (value.startsWith('@') && value.includes(':')) return {kind: 'identity', handle: value}
    if (value.startsWith('#')) return {kind: 'anon', name: value.slice(1)}
    return {kind: 'plain', text: value}
}

/**
 * Info-panel "Created by" field (feature 26 §8). Owned pictures edit the authoritative `creator`
 * (with "reset to owner default" when set); received pictures edit the recipient-local
 * `creator_override` (with "reset to original" when overridden). Editing uses {@link ContactInput}:
 * plain-text credits are free, a leading `@` autocompletes a real `@user:domain` contact (verified
 * against the resolver), and `#` is blocked (system-owned public-share sigil).
 */
export function CreatorField({picture}: { picture: PictureDetail }) {
    const owned = picture.owner_username == null
    const overridden = !owned && picture.creator_override != null
    const hasOwnedValue = owned && picture.creator_value != null
    const rawEditable = owned ? picture.creator_value : picture.creator_override

    const setCreator = useSetCreator(picture.id)
    const [editing, setEditing] = useState(false)
    const [value, setValue] = useState('')
    const [valid, setValid] = useState(true)

    const startEdit = () => {
        setValue(rawEditable ?? '')
        setValid(true)
        setEditing(true)
    }

    const save = () => {
        if (!valid) return
        const trimmed = value.trim()
        setCreator.mutate(
            {value: trimmed === '' ? null : trimmed, ...(owned ? {} : {mode: 'local' as const})},
            {onSuccess: () => setEditing(false)},
        )
    }
    const reset = () => setCreator.mutate({value: null, ...(owned ? {} : {mode: 'local' as const})})

    if (editing) {
        return (
            <div className="flex flex-col gap-1 px-3 pb-2">
                <div className="flex items-start gap-1">
                    <ContactInput
                        value={value}
                        onChange={setValue}
                        allowCustomValues
                        includeSelf
                        onValidityChange={setValid}
                        onEnter={save}
                        onEscape={() => setEditing(false)}
                        autoFocus
                        className="flex-1"
                    />
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0 text-muted-foreground hover:text-emerald-500"
                        title="Save"
                        disabled={!valid || setCreator.isPending}
                        onClick={save}
                    >
                        <Check className="h-3.5 w-3.5"/>
                    </Button>
                    <Button
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                        title="Cancel"
                        onClick={() => setEditing(false)}
                    >
                        <X className="h-3.5 w-3.5"/>
                    </Button>
                </div>
                <span className="text-[11px] text-muted-foreground">
                    {owned ? 'Leave empty to reset to the owner default.' : 'Leave empty to reset to the original credit.'}
                </span>
            </div>
        )
    }

    const parsed = parseCreator(picture.creator)
    const text = parsed.kind === 'anon' ? parsed.name : parsed.kind === 'identity' ? parsed.handle : parsed.text

    return (
        <div className="group flex items-center gap-1.5 px-3 pb-2 text-xs text-muted-foreground">
            <User className="h-3.5 w-3.5 shrink-0"/>
            <span className="min-w-0 truncate">
                Created by{' '}
                <button
                    onClick={startEdit}
                    className={cn(
                        'truncate text-left hover:text-foreground',
                        parsed.kind === 'identity' ? 'font-medium text-foreground' : 'text-foreground',
                    )}
                    title="Edit creator"
                >
                    {text}
                </button>
            </span>
            {parsed.kind === 'anon' && (
                <Tooltip>
                    <TooltipTrigger asChild>
                        <span
                            className="inline-flex shrink-0 items-center gap-0.5 rounded bg-sky-500/15 px-1 text-[10px] font-medium leading-4 text-sky-500">
                            <Share2 className="h-2.5 w-2.5"/>
                            public share
                        </span>
                    </TooltipTrigger>
                    <TooltipContent side="left" className="max-w-[15rem] text-xs">
                        This credit was entered by an anonymous contributor through a public share.
                    </TooltipContent>
                </Tooltip>
            )}
            {overridden && (
                <Tooltip>
                    <TooltipTrigger asChild>
                        <span className="shrink-0 rounded bg-amber-500/15 px-1 text-[10px] font-medium leading-4 text-amber-500">
                            overridden
                        </span>
                    </TooltipTrigger>
                    <TooltipContent side="left" className="max-w-[15rem] text-xs">
                        You relabelled the creator locally. This is private to you and never propagates —
                        others still see “{picture.creator_origin}”.
                    </TooltipContent>
                </Tooltip>
            )}
            {(hasOwnedValue || overridden) && (
                <Button
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 shrink-0 opacity-0 transition-opacity group-hover:opacity-100"
                    title={owned ? 'Reset to owner default' : 'Reset to original'}
                    disabled={setCreator.isPending}
                    onClick={reset}
                >
                    <RotateCcw className="h-3 w-3"/>
                </Button>
            )}
        </div>
    )
}
