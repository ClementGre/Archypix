import {useState} from 'react'
import {Info} from 'lucide-react'
import {Button} from '@/components/ui/button'
import {Popover, PopoverContent, PopoverTrigger} from '@/components/ui/popover'
import {formatDateTime, TagPath} from '@/lib/utils'
import type {PublicShareSummary} from '@/api/publicShares'
import {DetailRow, FlagChip} from './ShareInfoPopover'

/**
 * Details for a public share link, mirroring `ShareInfoPopover` (opens on hover / tap, anchored right).
 * Surfaces the permissions, gating, counts, and timestamps that don't fit on the compact card.
 */
export function PublicShareInfoPopover({share}: { share: PublicShareSummary }) {
    const [open, setOpen] = useState(false)
    const p = share.permissions

    return (
        <Popover open={open} onOpenChange={setOpen}>
            <PopoverTrigger asChild>
                <Button
                    size="icon"
                    variant="ghost"
                    className="h-7 w-7 text-muted-foreground hover:text-foreground"
                    title="Details"
                    onMouseEnter={() => setOpen(true)}
                >
                    <Info className="h-3.5 w-3.5"/>
                </Button>
            </PopoverTrigger>
            <PopoverContent
                side="right"
                align="start"
                className="max-h-[70vh] w-72 space-y-2 overflow-y-auto p-3"
                onMouseLeave={() => setOpen(false)}
            >
                <p className="min-w-0 break-words text-sm font-medium">{share.name}</p>

                <div className="flex flex-wrap gap-1">
                    <FlagChip label="Originals" on={p.allow_originals}/>
                    <FlagChip label="Uploads" on={p.allow_upload}/>
                    <FlagChip label="Share-back" on={p.allow_share_back}/>
                    <FlagChip label="EXIF editing" on={p.conv_allow_exif_edit}/>
                    <FlagChip label="Future additions" on={p.conv_future}/>
                </div>

                <div className="space-y-0.5">
                    <DetailRow label="Shared tag">{TagPath.toDisplay(share.tag_path)}</DetailRow>
                    <DetailRow label="Password">{share.has_password ? 'Yes' : 'No'}</DetailRow>
                    <DetailRow label="Created">{formatDateTime(share.created_at)}</DetailRow>
                    {share.expires_at && <DetailRow label="Expires">{formatDateTime(share.expires_at)}</DetailRow>}
                    <DetailRow label="Derived shares">{share.derived_share_count}</DetailRow>
                    <DetailRow label="Contributions">{share.contribution_count}</DetailRow>
                </div>

                {share.message ? (
                    <p className="whitespace-pre-wrap break-words text-xs text-muted-foreground">{share.message}</p>
                ) : (
                    <p className="text-xs italic text-muted-foreground/60">No message</p>
                )}
            </PopoverContent>
        </Popover>
    )
}
