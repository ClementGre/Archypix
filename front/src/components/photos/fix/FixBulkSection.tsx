// The multi-selection fix pane (feature 30 §8), inserted into the batch details panel before Tags.
// Computes provenance-carrying per-target rows then opens the bulk preview, or hands off to reference
// picking. Non-collapsible, matching the single fix pane.

import {useState} from 'react'
import {useQueryClient} from '@tanstack/react-query'
import {Loader2, Wrench} from 'lucide-react'
import {getPicture} from '@/api/pictures'
import {queryKeys} from '@/lib/constants'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useReferencePhase} from '@/hooks/useReferencePhase'
import {isMemberSelected, useSelectionStore} from '@/stores/selection'
import {useGridItems} from '@/stores/gridItems'
import {Button} from '@/components/ui/button'
import {FixBulkDialog} from './FixBulkDialog'
import {FixPane, PickReferencesButton} from './fixShared'
import {type BulkRow, dateBulkRows, gpsBulkRows} from '@/lib/fixBulk'
import type {PictureListItem} from '@/lib/types'

export function FixBulkSection() {
    const {params} = useGalleryParams()
    const field = params.fix
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const clear = useSelectionStore((s) => s.clear)
    const grid = useGridItems((s) => s.items)
    const queryClient = useQueryClient()
    const {begin} = useReferencePhase()
    const [rows, setRows] = useState<BulkRow[] | null>(null)
    const [loading, setLoading] = useState(false)

    if (!field) return null

    const items: PictureListItem[] = query
        ? grid.filter((i) => isMemberSelected(query, includeIds, excludeIds, i.id))
        : includeIds.map((id) => grid.find((i) => i.id === id)).filter((i): i is PictureListItem => !!i)

    if (items.length < 2) return null
    const hasReceived = items.some((i) => !i.owned)

    const openPreview = async () => {
        if (field === 'date') {
            setRows(dateBulkRows(items))
            return
        }
        setLoading(true)
        const computed = await gpsBulkRows(items, grid, (id) =>
            queryClient.fetchQuery({queryKey: queryKeys.picture(id), queryFn: () => getPicture(id)}),
        )
        setRows(computed)
        setLoading(false)
    }

    return (
        <FixPane title={field === 'gps' ? 'Fix location' : 'Fix date'}>
            <div className="flex items-center gap-2 rounded-md border border-border bg-muted/40 p-2 text-xs">
                <Wrench className="h-3.5 w-3.5 shrink-0 text-muted-foreground"/>
                <span>{items.length} photos selected to fix {field === 'gps' ? 'GPS' : 'the date'}.</span>
            </div>
            <Button size="sm" className="w-full gap-1.5" disabled={loading} onClick={openPreview}>
                {loading ? <Loader2 className="h-4 w-4 animate-spin"/> : null}
                Preview &amp; apply {field === 'gps' ? 'interpolated GPS' : 'dates'}
            </Button>
            <PickReferencesButton onClick={() => begin(field, items.map((i) => i.id))}/>

            {rows && (
                <FixBulkDialog
                    open
                    onOpenChange={(o) => !o && setRows(null)}
                    field={field}
                    title={`Fix ${field === 'gps' ? 'GPS' : 'dates'} for ${items.length} photos`}
                    initialRows={rows}
                    hasReceived={hasReceived}
                    onApplied={clear}
                />
            )}
        </FixPane>
    )
}
