import {useCallback, useEffect, useMemo, useState} from 'react'
import {useMutation, useQuery, useQueryClient} from '@tanstack/react-query'
import {toast} from 'sonner'
import {
    batchEditTags,
    type BatchEditTagsBody,
    deleteTagMeta,
    listAllTags,
    listAllTagsWithSources,
    listPictureTags,
    renameTag,
} from '@/api/tags'
import {queryKeys} from '@/lib/constants'
import {invalidatePicturesAndTags} from '@/lib/invalidation'
import {
    allPendingTagMeta,
    flushTagMeta,
    installTagMetaFlushHandlers,
    onTagMetaQueueChange,
    queueTagMeta,
    setTagMetaQueueErrorHandler,
} from '@/lib/tagMetaQueue'
import {buildTagTree, type TagNode, type TrashView} from '@/lib/tagTree'
import type {TagListItem, TagMeta, TagMetaPatch} from '@/lib/types'

/** The tag tree is eventually consistent (the pipeline is async), so a periodic refetch is enough
 *  — local mutations invalidate immediately (feature 34 §4). */
const REFETCH_MS = 5 * 60_000

/** Re-render whenever the write queue changes, so the overlay below is re-applied. */
function usePendingTagMetaVersion(): number {
    const [version, setVersion] = useState(0)
    useEffect(() => onTagMetaQueueChange(() => setVersion((n) => n + 1)), [])
    return version
}

/**
 * The one app-start payload (§4): every tag with its counts, derived dates and metadata. Expanding,
 * reordering, switching view mode or grouping, and navigating between tags issue no further tag
 * queries.
 *
 * Unflushed writes are **overlaid on the served payload** rather than written into the cache: the
 * queue debounces for 60 s, the tree invalidates this query on most interactions, and a refetch
 * landing in between would otherwise serve the pre-change row and silently revert the user.
 */
export function useAllTags() {
    const version = usePendingTagMetaVersion()
    const select = useCallback(
        (items: TagListItem[]) => allPendingTagMeta().reduce(applyPatch, items),
        // eslint-disable-next-line react-hooks/exhaustive-deps
        [version],
    )
    return useQuery({
        queryKey: queryKeys.tags(),
        queryFn: listAllTags,
        refetchInterval: REFETCH_MS,
        select,
    })
}

/** The provenance breakdown — the path×source query is too heavy for the app-start payload, so the
 *  edit dialog fetches it on open (§4). */
export function useAllTagsWithSources(enabled: boolean) {
    return useQuery({
        queryKey: [...queryKeys.tags(), 'sources'],
        queryFn: listAllTagsWithSources,
        enabled,
    })
}

/** The resolved tree for the active trash view, plus a path → metadata index for the flat surfaces
 *  (chips, breadcrumbs, pickers) that need a display name without walking the tree. */
export function useTagTree(trash: TrashView = 'exclude'): {
    tree: TagNode[]
    metaByPath: Map<string, TagMeta>
    items: TagListItem[]
    isPending: boolean
    isError: boolean
    error: unknown
} {
    const {data, isPending, isError, error} = useAllTags()
    const items = useMemo(() => data ?? [], [data])
    const tree = useMemo(() => buildTagTree(items, trash), [items, trash])
    const metaByPath = useMemo(() => {
        const map = new Map<string, TagMeta>()
        for (const i of items) if (i.meta) map.set(i.path, i.meta)
        return map
    }, [items])
    return {tree, metaByPath, items, isPending, isError, error}
}

/** The root row (`tag_path = ''`) — view mode, grouping, child ordering and subtag placement for
 *  the no-tag gallery view, on the same code path as every other node (§3.3). */
export function useRootTagMeta(): TagMeta | null {
    const {data} = useAllTags()
    return data?.find((i) => i.path === '')?.meta ?? null
}

/**
 * Queue a metadata write (§4.1): the overlay in `useAllTags` makes it visible at once, so the UI
 * never waits on the debounce. Pass only the fields that changed.
 */
export function useWriteTagMeta() {
    return (patch: TagMetaPatch) => queueTagMeta(patch)
}

/** Merge one queued patch onto the served payload. */
function applyPatch(items: TagListItem[], patch: TagMetaPatch): TagListItem[] {
    const {tag_path, ...fields} = patch
    const base: TagMeta = {
        tag_path,
        display_name: null,
        description: null,
        cover_picture_id: null,
        color: null,
        date_from: null,
        date_to: null,
        show_when_empty: false,
        sort_index: null,
        children_order: 'manual',
        view_mode: 'subtag',
        subtag_placement: null,
        grouping: {},
        webdav_dir_name: null,
    }
    const found = items.find((i) => i.path === tag_path)
    const merged: TagMeta = {...(found?.meta ?? base), ...fields, tag_path}
    if (found) return items.map((i) => (i.path === tag_path ? {...i, meta: merged} : i))
    // A brand-new empty tag has no entry yet; synthesize one so the tree shows it at once.
    return [
        ...items,
        {path: tag_path, count: 0, exact_count: 0, date_from: null, date_to: null, meta: merged},
    ]
}

/** *Reset metadata* / *Delete tag* (§6). Unlike the queued writes this is immediate — there is no
 *  undo, so the user has already confirmed. */
export function useResetTagMeta() {
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: async (tagPaths: string[]) => {
            // Anything still queued for these tags would resurrect the row after the delete.
            await flushTagMeta()
            await deleteTagMeta(tagPaths)
        },
        onSuccess: () => void queryClient.invalidateQueries({queryKey: queryKeys.tags()}),
    })
}

/** Install the unload/visibility flush triggers and the failed-flush toast once, at app level. */
export function useTagMetaQueue(): void {
    useEffect(() => {
        setTagMetaQueueErrorHandler((message) => toast.error(message))
        return installTagMetaFlushHandlers()
    }, [])
}

export function usePictureTags(pictureId: string | null) {
    return useQuery({
        queryKey: queryKeys.pictureTags(pictureId ?? ''),
        enabled: !!pictureId,
        queryFn: () => listPictureTags(pictureId!),
    })
}

export function useBatchEditTags() {
    const queryClient = useQueryClient()
    return useMutation({
        mutationFn: (body: BatchEditTagsBody) => batchEditTags(body),
        onSuccess: () => invalidatePicturesAndTags(queryClient),
    })
}

/**
 * Rename a tag subtree (edge case §7). The cascade also rewrites shares, tagging services,
 * hierarchies and the tag's own metadata row, so their caches are invalidated alongside
 * pictures + tags.
 */
export function useRenameTag() {
    const queryClient = useQueryClient()
    return useMutation({
        // A queued write for the old path would be re-applied under a path that no longer exists.
        mutationFn: async ({oldTag, newTag}: { oldTag: string; newTag: string }) => {
            await flushTagMeta()
            return renameTag(oldTag, newTag)
        },
        onSuccess: () => {
            const run = () => {
                void queryClient.invalidateQueries({queryKey: ['pictures']})
                void queryClient.invalidateQueries({queryKey: ['tags']})
                void queryClient.invalidateQueries({queryKey: ['tagging']})
                void queryClient.invalidateQueries({queryKey: ['hierarchies']})
                void queryClient.invalidateQueries({queryKey: ['shares']})
            }
            run()
            setTimeout(run, 2000)
            setTimeout(run, 6000)
        },
    })
}
