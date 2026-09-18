// The view root's resolved preferences (feature 35 §4, §7). View mode, grouping and subtag
// placement live in `tag_metadata`, not the URL, so they follow the user across devices — the cost
// is that the header has to state the active mode whenever it is not the default.

import {useCallback, useMemo} from 'react'
import {useGalleryParams} from '@/hooks/useGalleryParams'
import {useSettings} from '@/hooks/useSettings'
import {useTagTree, useWriteTagMeta} from '@/hooks/useTags'
import {type BucketContext, type GroupingKind, NO_GROUPING, resolveGrouping, withGrouping} from '@/lib/grouping'
import {resolvePlacement, resolveViewMode, tagArmFor} from '@/lib/timeline'
import type {PictureFilter, TagSubtagPlacement, TagViewMode} from '@/lib/types'

export interface TimelineView {
    /** The tag whose subtree structures the view; `''` is the root (34 §3.3). */
    root: string
    mode: TagViewMode
    grouping: GroupingKind
    placement: TagSubtagPlacement
    /** Everything the bucketers need beyond the item itself. */
    bucketCtx: BucketContext
    /** Fix mode pins the view flat, and says so in the header (§9). */
    pinned: boolean
    /** True while the structured renderer is in charge (i.e. not hierarchy browse). */
    structured: boolean
    /** ⌘A / "select all" over the whole view, layered with `inc`/`exc` (§7). */
    selectionFilter: PictureFilter
    setMode: (mode: TagViewMode) => void
    setGrouping: (kind: GroupingKind) => void
    setPlacement: (placement: TagSubtagPlacement) => void
}

export function useTimelineView(): TimelineView {
    const {params, scopeFilter} = useGalleryParams()
    const {metaByPath} = useTagTree(params.trash)
    const {data: settings} = useSettings()
    const write = useWriteTagMeta()

    const root = params.tag ?? ''
    const meta = metaByPath.get(root) ?? null
    const browsing = !!params.hierarchy
    // Fix mode wants a flat date-sorted stream to find neighbours; grouping wants structure (§9).
    const pinned = !!params.fix

    // Fix mode only rules out `subtag`: it needs a flat stream to find neighbours, but narrowing to
    // one tag's own photos keeps the stream flat and is often exactly what you want to fix.
    const stored = resolveViewMode(meta)
    const mode = pinned && stored === 'subtag' ? 'all' : stored
    const grouping: GroupingKind =
        pinned || browsing ? NO_GROUPING : resolveGrouping(meta, params.sort)
    const placement = resolvePlacement(meta, root, params.sort)

    const bucketCtx: BucketContext = useMemo(
        () => ({hemisphere: settings?.hemisphere ?? 'north', nearTime: params.nearTime}),
        [settings?.hemisphere, params.nearTime],
    )

    const selectionFilter: PictureFilter = useMemo(() => {
        if (params.hierarchy) {
            return {kind: 'hierarchy', hierarchy_id: params.hierarchy, path: params.hpath, ...scopeFilter}
        }
        const arm = tagArmFor(mode, root)
        const include = [...(arm.include_tags ?? []), ...params.include]
        return {
            ...scopeFilter,
            kind: 'flat',
            include_tags: include.length ? include : undefined,
            exclude_tags: params.exclude.length ? params.exclude : undefined,
            exact: arm.exact,
            untagged: arm.untagged,
            match: 'all',
        }
    }, [params.hierarchy, params.hpath, params.include, params.exclude, scopeFilter, mode, root])

    const setMode = useCallback(
        (next: TagViewMode) => write({tag_path: root, view_mode: next}),
        [write, root],
    )
    const setGrouping = useCallback(
        (kind: GroupingKind) =>
            write({tag_path: root, grouping: withGrouping(meta?.grouping ?? {}, params.sort, kind)}),
        [write, root, meta?.grouping, params.sort],
    )
    const setPlacement = useCallback(
        (next: TagSubtagPlacement) => write({tag_path: root, subtag_placement: next}),
        [write, root],
    )

    return {
        root,
        mode,
        grouping,
        placement,
        bucketCtx,
        pinned,
        structured: !browsing,
        selectionFilter,
        setMode,
        setGrouping,
        setPlacement,
    }
}
