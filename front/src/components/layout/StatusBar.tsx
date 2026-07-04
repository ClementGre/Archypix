import {useLocation, useNavigate} from 'react-router-dom'
import {HardDrive, Images, LayoutGrid, Wand2} from 'lucide-react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useSelectionCount} from '@/hooks/useAggregate'
import {useTaggingServices} from '@/hooks/useTaggingServices'
import {useStorage} from '@/hooks/useSettings'
import {StorageBar} from '@/components/StorageBar'
import {ROW_HEIGHT_MAX, ROW_HEIGHT_MIN, useUIStore} from '@/stores/ui'
import {formatBytes, TagPath} from '@/lib/utils'

/** Thin footer with user/storage stats, the current view, selection, and the thumbnail-size slider. */
export function StatusBar() {
    const {pathname} = useLocation()
    const navigate = useNavigate()
    const isGallery = pathname === '/'

    const {params} = useGalleryParams()
    const {count: selectedCount} = useSelectionCount()
    const rowHeight = useUIStore((s) => s.rowHeight)
    const setRowHeight = useUIStore((s) => s.setRowHeight)
    const {data: services} = useTaggingServices()
    const {data: storage} = useStorage()

    const used = storage?.used_bytes ?? 0
    const quota = storage?.quota_bytes ?? null
    const storagePct =
        quota && quota > 0 ? Math.min(100, Math.round((used / quota) * 100)) : 0
    const storageTitle =
        quota && quota > 0
            ? `Storage: ${formatBytes(used)} of ${formatBytes(quota)} used (${storagePct}%)`
            : `Storage: ${formatBytes(used)} used (unlimited)`
    const viewLabel = params.tag ? TagPath.toDisplay(params.tag) : 'All photos'

    return (
        <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-border bg-card px-3 text-[11px] text-muted-foreground">
            {/* Storage usage (feature 22): segmented bar (originals/versions/trashed). Quota total only shown when set. */}
            <button
                onClick={() => navigate('/settings')}
                className="flex items-center gap-1.5 transition-colors hover:text-foreground"
                title={storageTitle}
            >
                <HardDrive className="h-3 w-3"/>
                <span className="tabular-nums">{formatBytes(used)}</span>
                {storage && (
                    <>
                        <StorageBar
                            breakdown={storage.breakdown}
                            quotaBytes={quota}
                            usedBytes={used}
                            className="w-20 rounded-full"
                        />
                        {quota && quota > 0 && <span className="tabular-nums">{formatBytes(quota)}</span>}
                    </>
                )}
            </button>

            <button
                onClick={() => navigate('/tagging')}
                className="hidden items-center gap-1 transition-colors hover:text-foreground sm:flex"
                title="Tagging services"
            >
                <Wand2 className="h-3 w-3"/>
                <span className="tabular-nums">{services?.length ?? 0}</span> services
            </button>

            <div className="ml-auto flex items-center gap-3">
                {isGallery && (
                    <span className="hidden items-center gap-1 sm:flex" title="Current view">
                        <Images className="h-3 w-3"/>
                        <span className="max-w-[14rem] truncate">{viewLabel}</span>
                    </span>
                )}
                {isGallery && selectedCount > 0 && (
                    <span className="font-medium text-foreground tabular-nums">{selectedCount} selected</span>
                )}
                {isGallery && (
                    <label className="flex items-center gap-1.5" title="Thumbnail size">
                        <LayoutGrid className="h-3 w-3"/>
                        <input
                            type="range"
                            min={ROW_HEIGHT_MIN}
                            max={ROW_HEIGHT_MAX}
                            step={10}
                            value={rowHeight}
                            onChange={(e) => setRowHeight(Number(e.target.value))}
                            className="h-1 w-30 cursor-pointer accent-primary"
                            aria-label="Thumbnail size"
                        />
                    </label>
                )}
            </div>
        </footer>
    )
}
