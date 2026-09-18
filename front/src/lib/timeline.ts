// Timeline view resolution (feature 35 §3–§5). One rule, applied at any tag: render T's direct
// photos and T's child tags as one stream, recursing into whatever a child remembers for itself.

import {isDateField} from '@/lib/grouping'
import {effectiveDate, type TagNode, type TrashView} from '@/lib/tagTree'
import type {SortField, SortOrder, TagMeta, TagSubtagPlacement, TagViewMode} from '@/lib/types'

export const DEFAULT_VIEW_MODE: TagViewMode = 'subtag'

export function resolveViewMode(meta: TagMeta | null | undefined): TagViewMode {
    return meta?.view_mode ?? DEFAULT_VIEW_MODE
}

/**
 * Where a parent's child blocks sit (34 §3.4). `null` is derived, and the derived default is not
 * uniform: the root's children are namespaces spanning every month, so scattering them would be
 * noise. Under a non-date sort a tag has no value to place against, so `top` wins whatever is stored.
 */
export function resolvePlacement(
    meta: TagMeta | null | undefined,
    tagPath: string,
    sort: SortField,
): TagSubtagPlacement {
    if (!isDateField(sort)) return 'top'
    const stored = meta?.subtag_placement
    if (stored) return stored
    return tagPath === '' ? 'top' : 'in_sections'
}

/** The endpoint that comes **first in the active sort order** — where the eye looks for it (§5). */
export function placingDate(node: TagNode, order: SortOrder, trash: TrashView): string | null {
    return effectiveDate(node, order === 'desc' ? 'date_to' : 'date_from', trash)
}

/**
 * The tag arm of every query and of `selectionFilter` (§7). `subtag` and `all` share it, so toggling
 * between them preserves the selection; `direct` changes what the view contains and correctly clears
 * it through `PhotoGrid`'s existing `filterSig` effect.
 */
export interface TagArm {
    include_tags?: string[]
    exact?: string[]
    untagged?: boolean
}

export function tagArmFor(mode: TagViewMode, tagPath: string): TagArm {
    if (mode === 'direct') {
        return tagPath === '' ? {untagged: true} : {exact: [tagPath]}
    }
    return tagPath === '' ? {} : {include_tags: [tagPath]}
}

/** Blocks with no date at all flush at the end of the stream, after the last section (§10.4). */
export function partitionUndated(
    children: TagNode[],
    order: SortOrder,
    trash: TrashView,
): { dated: TagNode[]; undated: TagNode[] } {
    const dated: TagNode[] = []
    const undated: TagNode[] = []
    for (const c of children) (placingDate(c, order, trash) ? dated : undated).push(c)
    return {dated, undated}
}
