import {useState} from 'react'
import {Pencil, User} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'
import {ContactInput} from '@/components/common/ContactInput'
import {BatchConfirmDialog} from './BatchConfirmDialog'
import {batchSetCreator} from '@/api/pictures'
import {useBatchMutations} from '@/hooks/useBatch'
import type {toApiSelection} from '@/stores/selection'
import type {FieldAggregate} from '@/lib/types'

/** Display a creator credit (feature 26 §3), stripping the anonymous-uploader `#` sigil. */
function creatorLabel(value: string): string {
    return value.startsWith('#') ? value.slice(1) : value
}

/**
 * Batch "Created by" control for the multi-select panel (feature 26 integration): shows the resolved
 * creator across the selection (a common value, or "Mixed" with the distinct list) and edits it for
 * the whole selection — owned photos get the authoritative `creator` (re-announced), received photos
 * the recipient-local `creator_override`. Empty resets/clears. Reuses {@link ContactInput}.
 */
export function BatchCreatorControl({creator, total, selection}: {
    creator?: FieldAggregate
    total: number
    selection: ReturnType<typeof toApiSelection>
}) {
    const {creator: mutation} = useBatchMutations()
    const [open, setOpen] = useState(false)
    const [value, setValue] = useState('')
    const [valid, setValid] = useState(true)

    if (!creator || creator.type !== 'distinct') return null

    const common = typeof creator.common === 'string' ? creator.common : null
    const values = creator.distinct.map((d) => String(d.value))
    const mixed = common == null && values.length > 0

    const openEdit = () => {
        setValue('')
        setValid(true)
        setOpen(true)
    }
    const apply = () => {
        const trimmed = value.trim()
        mutation.mutate({selection, value: trimmed === '' ? null : trimmed})
    }

    return (
        <div className="flex items-center justify-between gap-2 pt-1">
            <div className="flex min-w-0 items-center gap-1.5 text-muted-foreground">
                <User className="h-3.5 w-3.5 shrink-0"/>
                {common != null ? (
                    <span className="truncate text-foreground" title={common}>{creatorLabel(common)}</span>
                ) : mixed ? (
                    <Tooltip delayDuration={0}>
                        <TooltipTrigger asChild>
                            <span className="text-foreground">Mixed <span className="tabular-nums">({values.length})</span></span>
                        </TooltipTrigger>
                        <TooltipContent className="max-w-[16rem] text-xs">
                            {creator.distinct.slice(0, 8).map((d, i) => (
                                <div key={i} className="truncate">
                                    {creatorLabel(String(d.value))} <span className="tabular-nums opacity-70">×{d.count}</span>
                                </div>
                            ))}
                            {creator.distinct_overflow > 0 && <div className="opacity-70">+{creator.distinct_overflow} more</div>}
                        </TooltipContent>
                    </Tooltip>
                ) : (
                    <span className="text-muted-foreground">—</span>
                )}
            </div>
            <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 shrink-0"
                title="Set creator for all"
                disabled={total === 0}
                onClick={openEdit}
            >
                <Pencil className="h-3 w-3"/>
            </Button>

            <BatchConfirmDialog
                open={open}
                onOpenChange={setOpen}
                title="Set creator?"
                confirmLabel="Set creator"
                confirmDisabled={!valid}
                dryRun={() => batchSetCreator({selection, value: null, dry_run: true})}
                renderResult={(r) => (
                    <span>
                        Sets the creator on <span className="font-medium tabular-nums">{r.edited ?? 0}</span> owned and{' '}
                        <span className="font-medium tabular-nums">{r.local_override ?? 0}</span> received of {total} photos.
                    </span>
                )}
                onConfirm={apply}
            >
                <div className="flex flex-col gap-1.5 py-1">
                    <ContactInput
                        value={value}
                        onChange={setValue}
                        allowCustomValues
                        includeSelf
                        onValidityChange={setValid}
                        placeholder="Creator credit (name or @user:domain)"
                        className="w-full"
                    />
                    <span className="text-[11px] text-muted-foreground">
                        Leave empty to reset owned photos to the owner default and clear received overrides.
                    </span>
                </div>
            </BatchConfirmDialog>
        </div>
    )
}
