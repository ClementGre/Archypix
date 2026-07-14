import {useCallback, useMemo} from 'react'
import {useSearchParams} from 'react-router-dom'
import type {PictureFilter, PictureFilters, SortField, SortOrder, TrashFilter} from '@/lib/types'

export type Scope = 'all' | 'owned' | 'shared'
export type LeftPanelTab = 'tags' | 'incoming' | 'outgoing' | 'hierarchies'

/** Decoded view of the gallery's URL state. */
export interface GalleryParams {
    tag: string | null
    /** Additional include tags (wire form) layered on `tag` via the sidebar menu (`inc`). */
    include: string[]
    /** Exclude tags (wire form) (`exc`). */
    exclude: string[]
    /** Exact / strict include tags (wire form), no descendants (`exa`). */
    exact: string[]
    scope: Scope
    /** Trash-membership state: `exclude` (default) | `include` | `only` (trash view). */
    trash: TrashFilter
    sort: SortField
    order: SortOrder
    capturedAfter: string | null
    capturedBefore: string | null
    panel: LeftPanelTab
    /** Incoming share to highlight in the left panel (transient cross-link). */
    share: string | null
    /** Active hierarchy id — when set, the center grid browses it instead of the flat list. */
    hierarchy: string | null
    /** Directory path within the active hierarchy (slash-separated names, '' = root). */
    hpath: string
    /** Hierarchy id whose config editor occupies the center view (overrides the grid). */
    hedit: string | null
}

/** Patch applied to the URL state; omitted keys are left unchanged. */
export interface GalleryParamsPatch {
    tag?: string | null
    include?: string[]
    exclude?: string[]
    exact?: string[]
    scope?: Scope
    trash?: TrashFilter
    sort?: SortField
    order?: SortOrder
    capturedAfter?: string | null
    capturedBefore?: string | null
    panel?: LeftPanelTab
    share?: string | null
    hierarchy?: string | null
    hpath?: string
    hedit?: string | null
}

const DEFAULT_SORT: SortField = 'captured_at'
const DEFAULT_ORDER: SortOrder = 'desc'

/** Decode a comma-separated URL list into a trimmed, non-empty array. */
const splitList = (raw: string | null): string[] =>
    raw ? raw.split(',').map((s) => s.trim()).filter(Boolean) : []
/** Encode an array back to a comma list (or `null` to drop the param when empty). */
const joinList = (xs: string[] | undefined): string | null => (xs && xs.length ? xs.join(',') : null)

/**
 * Single source of truth for the gallery view: all filters, sort, search query
 * and the active left-panel tab live in the URL so the view is shareable and
 * survives back/forward navigation.
 */
export function useGalleryParams() {
    const [sp, setSp] = useSearchParams()

    const params: GalleryParams = useMemo(
        () => ({
            tag: sp.get('tag'),
            include: splitList(sp.get('inc')),
            exclude: splitList(sp.get('exc')),
            exact: splitList(sp.get('exa')),
            scope: (sp.get('scope') as Scope) || 'all',
            trash: (sp.get('trash') as TrashFilter) || 'exclude',
            sort: (sp.get('sort') as SortField) || DEFAULT_SORT,
            order: (sp.get('order') as SortOrder) || DEFAULT_ORDER,
            capturedAfter: sp.get('after'),
            capturedBefore: sp.get('before'),
            panel: (sp.get('panel') as LeftPanelTab) || 'tags',
            share: sp.get('share'),
            hierarchy: sp.get('hierarchy'),
            hpath: sp.get('hpath') ?? '',
            hedit: sp.get('hedit'),
        }),
        [sp],
    )

    const update = useCallback(
        (patch: GalleryParamsPatch, opts?: { replace?: boolean }) => {
            setSp(
                (prev) => {
                    const next = new URLSearchParams(prev)
                    const setOrDelete = (key: string, value: string | null | undefined, isDefault: boolean) => {
                        if (value == null || value === '' || isDefault) next.delete(key)
                        else next.set(key, value)
                    }
                    if ('tag' in patch) setOrDelete('tag', patch.tag, false)
                    if ('include' in patch) setOrDelete('inc', joinList(patch.include), false)
                    if ('exclude' in patch) setOrDelete('exc', joinList(patch.exclude), false)
                    if ('exact' in patch) setOrDelete('exa', joinList(patch.exact), false)
                    if ('scope' in patch) setOrDelete('scope', patch.scope, patch.scope === 'all')
                    if ('trash' in patch) setOrDelete('trash', patch.trash, patch.trash === 'exclude')
                    if ('sort' in patch) setOrDelete('sort', patch.sort, patch.sort === DEFAULT_SORT)
                    if ('order' in patch) setOrDelete('order', patch.order, patch.order === DEFAULT_ORDER)
                    if ('capturedAfter' in patch) setOrDelete('after', patch.capturedAfter, false)
                    if ('capturedBefore' in patch) setOrDelete('before', patch.capturedBefore, false)
                    if ('panel' in patch) setOrDelete('panel', patch.panel, patch.panel === 'tags')
                    if ('share' in patch) setOrDelete('share', patch.share, false)
                    if ('hierarchy' in patch) setOrDelete('hierarchy', patch.hierarchy, false)
                    if ('hpath' in patch) setOrDelete('hpath', patch.hpath, false)
                    if ('hedit' in patch) setOrDelete('hedit', patch.hedit, false)
                    return next
                },
                {replace: opts?.replace},
            )
        },
        [setSp],
    )

    const clearFilters = useCallback(() => {
        update(
            {
                tag: null,
                include: [],
                exclude: [],
                exact: [],
                scope: 'all',
                trash: 'exclude',
                sort: DEFAULT_SORT,
                order: DEFAULT_ORDER,
                capturedAfter: null,
                capturedBefore: null,
            },
            {replace: false},
        )
    }, [update])

    const filters: PictureFilters = useMemo(
        () => ({
            tag: params.tag,
            include: params.include,
            exclude: params.exclude,
            exact: params.exact,
            scope: params.scope,
            trash: params.trash,
            sort: params.sort,
            order: params.order,
            capturedAfter: params.capturedAfter,
            capturedBefore: params.capturedBefore,
        }),
        [params],
    )

    const hasActiveFilters =
        !!params.tag ||
        params.include.length > 0 ||
        params.exclude.length > 0 ||
        params.exact.length > 0 ||
        params.scope !== 'all' ||
        params.trash !== 'exclude' ||
        !!params.capturedAfter ||
        !!params.capturedBefore

    // The homogenized `PictureFilter` (feature 14 §3) describing the current view, for the
    // selection descriptor (`Ctrl+A` / "Select all").
    const selectionFilter: PictureFilter = useMemo(() => {
        const scope = {
            owned_only: params.scope === 'owned' || undefined,
            shared_with_me: params.scope === 'shared' || undefined,
            trash: params.trash !== 'exclude' ? params.trash : undefined,
            captured_after: params.capturedAfter ?? undefined,
            captured_before: params.capturedBefore ?? undefined,
        }
        if (params.hierarchy) {
            return {kind: 'hierarchy', hierarchy_id: params.hierarchy, path: params.hpath, ...scope}
        }
        // `tag` is the primary include; the sidebar menu layers extra include/exclude/exact tags.
        const include = [...(params.tag ? [params.tag] : []), ...params.include]
        return {
            kind: 'flat',
            include_tags: include.length ? include : undefined,
            exclude_tags: params.exclude.length ? params.exclude : undefined,
            exact: params.exact.length ? params.exact : undefined,
            match: 'all',
            ...scope,
        }
    }, [params])

    return {params, filters, update, clearFilters, hasActiveFilters, selectionFilter}
}
