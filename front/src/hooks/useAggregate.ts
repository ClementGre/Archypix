import {useMemo} from 'react'
import {keepPreviousData, useQuery, type UseQueryOptions} from '@tanstack/react-query'
import {aggregatePictures} from '@/api/pictures'
import {useDebouncedValue} from './useDebouncedValue'
import {toApiSelection, useSelectionStore} from '@/stores/selection'
import type {AggregateRequest, AggregateResponse, AggregateSection, PictureSelection} from '@/lib/types'

/**
 * Aggregate a selection (§4) for the multi-select panel. The descriptor is **debounced**
 * (§6.2) so e.g. un-checking ten pictures after a `Ctrl+A` issues one request, not ten;
 * `keepPreviousData` keeps the panel populated meanwhile. `sections` keeps it cheap — pass
 * only the sections whose foldable panels are open.
 */
export function useAggregate(
    selection: PictureSelection,
    sections: AggregateSection[],
    opts?: {
        enabled?: boolean
        tagProvenance?: boolean
        refetchInterval?: UseQueryOptions<AggregateResponse>['refetchInterval']
        delay?: number
    },
) {
    const signature = useMemo(
        () => JSON.stringify({selection, sections, p: opts?.tagProvenance ?? false}),
        [selection, sections, opts?.tagProvenance],
    )
    const debounced = useDebouncedValue(signature, opts?.delay ?? 350)
    const req = useMemo(
        () => JSON.parse(debounced) as { selection: PictureSelection; sections: AggregateSection[]; p: boolean },
        [debounced],
    )

    return useQuery<AggregateResponse>({
        queryKey: ['pictures', 'aggregate', debounced],
        enabled: opts?.enabled ?? true,
        placeholderData: keepPreviousData,
        refetchInterval: opts?.refetchInterval,
        queryFn: () => {
            const body: AggregateRequest = {selection: req.selection, sections: req.sections, tag_provenance: req.p}
            return aggregatePictures(body)
        },
    })
}

/**
 * Resolved count of the current selection. Explicit selections are counted client-side (zero
 * requests); a select-all reads the resolved `summary.count` (shared cache with the panel/status
 * bar, so it's one request total).
 */
export function useSelectionCount(): { count: number; isQuery: boolean; loading: boolean } {
    const query = useSelectionStore((s) => s.query)
    const includeIds = useSelectionStore((s) => s.includeIds)
    const excludeIds = useSelectionStore((s) => s.excludeIds)
    const selection = useMemo(
        () => toApiSelection({query, includeIds, excludeIds}),
        [query, includeIds, excludeIds],
    )
    const {data, isFetching} = useAggregate(selection, ['summary'], {enabled: query !== null})
    if (query === null) return {count: includeIds.length, isQuery: false, loading: false}
    return {count: data?.count ?? 0, isQuery: true, loading: isFetching && !data}
}
