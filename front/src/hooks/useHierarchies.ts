import {useInfiniteQuery, useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
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
import type {HierarchyConfig, PictureFilters, WebdavResponse} from '@/lib/types'

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
    opts?: { enabled?: boolean },
) {
    return useInfiniteQuery({
        queryKey: queryKeys.hierarchyBrowse(id ?? '', path, filters),
        enabled: !!id && (opts?.enabled ?? true),
        initialPageParam: 1,
        queryFn: ({pageParam}) =>
            browseHierarchy(id!, {
                path,
                page: pageParam as number,
                page_size: 50,
                thumbnail: 'medium',
                sort: filters.sort ?? 'ingested_at',
                order: filters.order ?? 'desc',
                ...(filters.scope === 'owned' ? {owned_only: true} : {}),
                ...(filters.scope === 'shared' ? {shared_with_me: true} : {}),
                ...(filters.includeDeleted ? {include_deleted: true} : {}),
                ...(filters.capturedAfter ? {captured_after: filters.capturedAfter} : {}),
                ...(filters.capturedBefore ? {captured_before: filters.capturedBefore} : {}),
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
