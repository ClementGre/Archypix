import {Images} from 'lucide-react'
import {ThumbnailSizeSlider} from '@/components/photos/ThumbnailSizeSlider'
import {useSelectionStore} from '@/stores/selection'
import {usePublicShare} from '@/components/public/context'

/** Thin public-share footer: a short share spec on the left, the shared zoom slider on the right. */
export function PublicStatusBar() {
    const {meta} = usePublicShare()
    const selectedCount = useSelectionStore((s) => s.includeIds.length)
    const perms = meta.permissions
    const spec = [
        meta.view_only ? 'view-only' : 'originals allowed',
        perms.allow_upload ? 'uploads open' : null,
    ]
        .filter(Boolean)
        .join(' · ')

    return (
        <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-border bg-card px-3 text-[11px] text-muted-foreground">
            <span className="flex min-w-0 items-center gap-1.5" title={meta.name}>
                <Images className="h-3 w-3 shrink-0"/>
                <span className="truncate">
                    {meta.picture_count} photo{meta.picture_count === 1 ? '' : 's'}
                    {spec && ` · ${spec}`}
                </span>
            </span>
            <div className="ml-auto flex items-center gap-3">
                {selectedCount > 0 && <span className="font-medium text-foreground tabular-nums">{selectedCount} selected</span>}
                <ThumbnailSizeSlider/>
            </div>
        </footer>
    )
}
