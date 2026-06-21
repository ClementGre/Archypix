import {PencilLine, X} from 'lucide-react'
import {Tooltip, TooltipContent, TooltipTrigger} from '@/components/ui/tooltip'

/**
 * Marks a received-picture EXIF field that the recipient has locally overridden. The override is
 * DB-only: it never touches the owner's file, so downloading the original (and WebDAV, which serves
 * the owner's file directly) still yields the owner's embedded value. The optional ✕ drops the
 * override so the owner's value flows through again.
 */
export function OverwrittenBadge({onRemove}: { onRemove?: () => void }) {
    return (
        <Tooltip>
            <TooltipTrigger asChild>
                <span className="inline-flex items-center gap-0.5 rounded bg-amber-500/15 px-1 text-[10px] font-medium leading-4 text-amber-500">
                    <PencilLine className="h-2.5 w-2.5"/>
                    overwritten
                    {onRemove && (
                        <button
                            onClick={(e) => {
                                e.stopPropagation()
                                onRemove()
                            }}
                            aria-label="Remove override"
                            className="ml-0.5 hover:text-amber-300"
                        >
                            <X className="h-2.5 w-2.5"/>
                        </button>
                    )}
                </span>
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-[15rem] text-xs">
                You overrode this field locally. The change is private to you — it is not written to the
                owner's file, so it is not visible in WebDAV (which serves the owner's picture directly,
                without applying your overrides).
            </TooltipContent>
        </Tooltip>
    )
}
