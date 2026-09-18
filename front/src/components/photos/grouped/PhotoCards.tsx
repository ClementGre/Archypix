// The card run shared by every stream in the grid (feature 35 §5). Extracted from `PhotoGrid` so
// the flat view and every nested section render identically; shift-click and the lightbox read the
// flattened order from `GroupedGridContext`, never from one section's own slice.

import type {MouseEvent} from 'react'
import {useSearchParams} from 'react-router-dom'
import {useQuery} from '@tanstack/react-query'
import {getPicture} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useIsMobile} from '@/hooks/useMediaQuery'
import {isMemberSelected, toApiSelection, useSelectionStore} from '@/stores/selection'
import {useTagDragStore} from '@/stores/tagDrag'
import {useFixHighlight} from '@/stores/fixHighlight'
import {useFixReference} from '@/stores/fixReference'
import {useUIStore} from '@/stores/ui'
import {useSettings} from '@/hooks/useSettings'
import {variantForSize} from '@/lib/utils'
import {useOrderedIds} from './GroupedGridContext'
import {PhotoCard} from '../PhotoCard'
import type {PictureListItem} from '@/lib/types'

/** The picture being fixed drives the grid's distance overlays (feature 30 §3). */
export function useFixTargetDetail() {
    const {params} = useGalleryParams()
    const referenceActive = useFixReference((s) => s.active)
    const fixTargetIds = useFixReference((s) => s.targetIds)
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)

    const fixTargetId = params.fix
        ? referenceActive
            ? fixTargetIds[0] ?? null
            : query === null && includeIds.length === 1 ? includeIds[0] : null
        : null
    const detail = useQuery({
        queryKey: queryKeys.picture(fixTargetId ?? ''),
        enabled: !!fixTargetId,
        queryFn: () => getPicture(fixTargetId!),
    }).data

    return {
        refTime: params.fix === 'gps' ? detail?.captured_at ?? null : null,
        geoRef:
            params.fix === 'date' && detail?.gps_lat != null && detail?.gps_lng != null
                ? {lat: detail.gps_lat, lng: detail.gps_lng}
                : null,
    }
}

export function PhotoCards({items, sourceTag}: { items: PictureListItem[]; sourceTag: string | null }) {
    const {params} = useGalleryParams()
    const getOrderedIds = useOrderedIds()
    const {data: settings} = useSettings()
    const rowHeight = useUIStore((s) => s.rowHeight)
    const openMobileDrawer = useUIStore((s) => s.openMobileDrawer)
    const isMobile = useIsMobile()
    const [, setSp] = useSearchParams()
    const {refTime} = useFixTargetDetail()

    const referenceActive = useFixReference((s) => s.active)
    const refIds = useFixReference((s) => s.refIds)
    const toggleRef = useFixReference((s) => s.toggleRef)
    const anchorIds = useFixHighlight((s) => s.anchorIds)

    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const multiSelect = useSelectionStore((s) => s.multiSelect)
    const select = useSelectionStore((s) => s.select)
    const toggle = useSelectionStore((s) => s.toggle)
    const selectTo = useSelectionStore((s) => s.selectTo)
    const enterMultiSelect = useSelectionStore((s) => s.enterMultiSelect)
    const clear = useSelectionStore((s) => s.clear)
    const startTagDrag = useTagDragStore((s) => s.start)
    const endTagDrag = useTagDragStore((s) => s.end)

    const handleSelect = (id: string) => (e: MouseEvent) => {
        e.stopPropagation()
        if (e.metaKey || e.ctrlKey) toggle(id)
        else if (e.shiftKey) selectTo(id, getOrderedIds())
        else if (multiSelect) toggle(id)
        else if (query === null && includeIds.length === 1 && includeIds[0] === id) clear()
        else {
            select(id)
            if (isMobile) openMobileDrawer('right')
        }
    }

    const handleLongPress = (id: string) => () => {
        if (multiSelect) toggle(id)
        else enterMultiSelect(id)
    }

    // A dragged card that is in the current selection drags the whole selection (34 §9).
    const handleDragStart = (id: string) => () => {
        const inSelection = isMemberSelected(query, includeIds, excludeIds, id)
        if (!inSelection) select(id)
        const s = useSelectionStore.getState()
        startTagDrag({
            selection: inSelection ? toApiSelection(s) : {include_ids: [id]},
            // 0 means "unknown" — a select-all over a query; the drop dialog uses the dry run.
            count: inSelection ? (s.query === null ? s.includeIds.length : 0) : 1,
            sourceTag,
        })
    }

    const openViewer = (id: string) =>
        setSp((prev) => {
            const next = new URLSearchParams(prev)
            next.set('view', id)
            return next
        })

    return (
        <>
            {items.map((it) => {
                // While picking references, only photos that HAVE the field being interpolated can be
                // a reference; the rest are dimmed + inert.
                const canRef = params.fix === 'gps' ? it.has_gps : params.fix === 'date' ? !!it.captured_at : false
                const refDisabled = referenceActive && !canRef
                return (
                    <PhotoCard
                        key={it.id}
                        item={it}
                        rowHeight={rowHeight}
                        selected={referenceActive ? false : isMemberSelected(query, includeIds, excludeIds, it.id)}
                        multiSelect={multiSelect}
                        showPurgeCountdown={params.trash === 'only'}
                        retentionDays={settings?.trash_retention_days ?? 30}
                        proximityRefTime={params.sort === 'time_near' ? params.nearTime : refTime}
                        fixMode={referenceActive ? null : params.fix}
                        dimmed={refDisabled}
                        anchorRole={
                            referenceActive
                                ? refIds.includes(it.id) ? params.fix : null
                                : params.fix === 'gps' && anchorIds.includes(it.id) ? 'gps' : null
                        }
                        onSelect={
                            referenceActive
                                ? (e) => {
                                    e.stopPropagation()
                                    if (canRef) toggleRef(it.id)
                                }
                                : handleSelect(it.id)
                        }
                        onLongPress={referenceActive ? () => {
                            if (canRef) toggleRef(it.id)
                        } : handleLongPress(it.id)}
                        onDragStart={referenceActive ? undefined : handleDragStart(it.id)}
                        onDragEnd={endTagDrag}
                        onOpen={() => openViewer(it.id)}
                    />
                )
            })}
        </>
    )
}

/** The thumbnail variant every stream requests, sized to the current zoom. */
export function useGridVariant() {
    const rowHeight = useUIStore((s) => s.rowHeight)
    return variantForSize(rowHeight)
}
