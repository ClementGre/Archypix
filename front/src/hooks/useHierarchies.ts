import {keepPreviousData, useInfiniteQuery, useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {
    browseHierarchy,
    createHierarchy,
    deleteHierarchy,
    getHierarchy,
    getHierarchyTree,
    getWebdav,
    listHierarchies,
    regenerateWebdavToken,
    setWebdavUseRedirect,
    updateHierarchy,
} from '@/api/hierarchies'
import {queryKeys} from '@/lib/constants'
import type {HierarchyConfig, PictureFilters, PictureVariant, WebdavResponse} from '@/lib/types'

export function useHierarchies() {
    return useQuery({queryKey: queryKeys.hierarchies(), queryFn: listHierarchies})
}

export function useHierarchy(id: string | null) {
    return useQuery({
        queryKey: queryKeys.hierarchy(id ?? ''),
        enabled: !!id,
        queryFn: () => getHierarchy(id!),
    })
}

/** Resolves one level of the directory tree at `path` (with picture counts; empty dirs hidden). */
export function useHierarchyTree(id: string | null, path: string, opts?: { enabled?: boolean }) {
    return useQuery({
        queryKey: queryKeys.hierarchyTree(id ?? '', path),
        enabled: !!id && (opts?.enabled ?? true),
        queryFn: () => getHierarchyTree(id!, {path, depth: 1, counts: true}),
    })
}

/** Paginated pictures of a hierarchy directory — same page shape as `usePictures`. */
export function useHierarchyBrowse(
    id: string | null,
    path: string,
    filters: PictureFilters,
    opts?: { enabled?: boolean; variant?: PictureVariant },
) {
    const variant = opts?.variant ?? 'medium'
    return useInfiniteQuery({
        queryKey: [...queryKeys.hierarchyBrowse(id ?? '', path, filters), variant],
        enabled: !!id && (opts?.enabled ?? true),
        placeholderData: keepPreviousData,
        initialPageParam: 1,
        queryFn: ({pageParam}) =>
            browseHierarchy(id!, {
                path,
                page: pageParam as number,
                page_size: 50,
                thumbnail: variant,
                sort: filters.sort ?? 'ingested_at',
                order: filters.order ?? 'desc',
                ...(filters.scope === 'owned' ? {owned_only: true} : {}),
                ...(filters.scope === 'shared' ? {shared_with_me: true} : {}),
                ...(filters.trash && filters.trash !== 'exclude' ? {trash: filters.trash} : {}),
                ...(filters.capturedAfter ? {captured_after: filters.capturedAfter} : {}),
                ...(filters.capturedBefore ? {captured_before: filters.capturedBefore} : {}),
                ...(filters.missingAny
                    ? {missing_any: true}
                    : {
                        ...(filters.gps && filters.gps !== 'any' ? {gps: filters.gps} : {}),
                        ...(filters.captureDate && filters.captureDate !== 'any'
                            ? {capture_date: filters.captureDate}
                            : {}),
                    }),
                ...(filters.sort === 'time_near' && filters.nearTime
                    ? {near_time: filters.nearTime}
                    : {}),
                ...(filters.sort === 'geo_near' && filters.nearLat != null && filters.nearLng != null
                    ? {near_lat: filters.nearLat, near_lng: filters.nearLng}
                    : {}),
            }),
        getNextPageParam: (last) =>
            last.page * last.page_size < last.total ? last.page + 1 : undefined,
    })
}

/** WebDAV mount info for a hierarchy — mints a token on first access (so only fetch when needed). */
export function useWebdav(id: string | null, opts?: { enabled?: boolean }) {
    return useQuery({
        queryKey: queryKeys.hierarchyWebdav(id ?? ''),
        enabled: !!id && (opts?.enabled ?? true),
        queryFn: () => getWebdav(id!),
    })
}

export function useWebdavMutations(id: string) {
    const qc = useQueryClient()
    const seed = (data: WebdavResponse) => qc.setQueryData(queryKeys.hierarchyWebdav(id), data)

    return {
        regenerate: useMutation({
            mutationFn: () => regenerateWebdavToken(id),
            onSuccess: seed,
        }),
        setUseRedirect: useMutation({
            mutationFn: (use_redirect: boolean) => setWebdavUseRedirect(id, use_redirect),
            onSuccess: seed,
        }),
    }
}

export function useHierarchyMutations() {
    const qc = useQueryClient()
    const invalidate = () => {
        void qc.invalidateQueries({queryKey: ['hierarchies']})
    }

    return {
        create: useMutation({mutationFn: createHierarchy, onSuccess: invalidate}),
        update: useMutation({
            mutationFn: (vars: {
                id: string
                body: { name?: string; enabled?: boolean; config?: HierarchyConfig }
            }) => updateHierarchy(vars.id, vars.body),
            onSuccess: invalidate,
        }),
        remove: useMutation({mutationFn: deleteHierarchy, onSuccess: invalidate}),
    }
}
