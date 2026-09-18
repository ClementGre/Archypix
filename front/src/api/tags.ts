import {apiClient, freshAccessToken} from './client'
import {useAuthStore} from '@/stores/auth'
import type {BatchDryRun, PictureSelection, PictureTagsWithSources, TagListItem, TagMetaPatch} from '@/lib/types'

/**
 * The whole browse read path (feature 34 §4): every tag path ancestor-expanded, with live and
 * trashed counts, derived date ranges and the metadata row. Fetched once at app start and on a
 * 5-minute interval — expanding, reordering, switching view mode and navigating issue no further
 * tag queries.
 */
export async function listAllTags(): Promise<TagListItem[]> {
    const {data} = await apiClient.get<{ tags: TagListItem[] }>('/api/authenticated/tags')
    return data.tags
}

/** The same payload plus the per-source provenance — too heavy for app start, so the edit dialog
 *  asks for it on open (§4). */
export async function listAllTagsWithSources(): Promise<TagListItem[]> {
    const {data} = await apiClient.get<{ tags: TagListItem[] }>('/api/authenticated/tags', {
        params: {with_sources: true},
    })
    return data.tags
}

/** Partial upsert of the decorative metadata (§11). Takes an array because the write queue flushes
 *  a coalesced batch; a single-item array is the common case. */
export async function upsertTagMeta(items: TagMetaPatch[]): Promise<void> {
    await apiClient.put('/api/authenticated/tags/meta', {items})
}

/** *Reset metadata* (§6): drop the rows, keep the tags. */
export async function deleteTagMeta(tagPaths: string[]): Promise<void> {
    await apiClient.delete('/api/authenticated/tags/meta', {data: {tag_paths: tagPaths}})
}

/**
 * The unload flush (§4.1). `fetch(keepalive)` rather than `navigator.sendBeacon`: auth is a
 * `Bearer` header, which `sendBeacon` cannot set, and `keepalive` survives document teardown
 * within a 64 KB body cap a preferences payload is nowhere near. Best-effort by construction — a
 * hard kill or an offline device can still drop it (§13.8).
 */
export async function upsertTagMetaKeepalive(items: TagMetaPatch[]): Promise<void> {
    const {backendUrl} = useAuthStore.getState()
    const token = await freshAccessToken()
    if (!backendUrl || !token) throw new Error('not authenticated')
    const res = await fetch(`${backendUrl}/api/authenticated/tags/meta`, {
        method: 'PUT',
        keepalive: true,
        headers: {'Content-Type': 'application/json', Authorization: `Bearer ${token}`},
        body: JSON.stringify({items}),
    })
    if (!res.ok) throw new Error(`tag metadata flush failed (${res.status})`)
}

export async function listPictureTags(pictureId: string): Promise<string[]> {
    const {data} = await apiClient.get<{ tags: string[] }>('/api/authenticated/tags', {
        params: {picture_id: pictureId},
    })
    return data.tags
}

export async function listPictureTagsWithSources(pictureId: string): Promise<PictureTagsWithSources> {
    const {data} = await apiClient.get<PictureTagsWithSources>('/api/authenticated/tags', {
        params: {picture_id: pictureId, with_sources: true},
    })
    return data
}

/**
 * Add/remove tags across a **selection** (§6.4). Accepts the selection descriptor or a legacy
 * explicit `picture_ids` list. Removal only affects `manual` rows. With `dry_run` returns the
 * §6.1 breakdown (`added`/`removed`); otherwise `{ ok, affected }`.
 */
export interface BatchEditTagsBody {
    selection?: PictureSelection
    picture_ids?: string[]
    add_tags?: string[]
    remove_tags?: string[]
    dry_run?: boolean
}

export async function batchEditTags(body: BatchEditTagsBody & { dry_run: true }): Promise<BatchDryRun>
export async function batchEditTags(body: BatchEditTagsBody): Promise<{ ok: true; affected: number }>
export async function batchEditTags(
    body: BatchEditTagsBody,
): Promise<{ ok: true; affected: number } | BatchDryRun> {
    const {data} = await apiClient.patch<{ ok: true; affected: number } | BatchDryRun>('/api/authenticated/tags', body)
    return data
}

/**
 * Rename a tag subtree everywhere it is referenced (edge case §7). Both paths are wire form. The
 * cascade (manual tags, shares, services, hierarchies) runs asynchronously; the response only acks.
 */
export async function renameTag(oldTag: string, newTag: string): Promise<{ ok: true }> {
    const {data} = await apiClient.post<{ ok: true }>('/api/authenticated/tags/rename', {
        old_tag: oldTag,
        new_tag: newTag,
    })
    return data
}
