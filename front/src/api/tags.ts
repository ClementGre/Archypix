import {apiClient} from './client'
import type {BatchDryRun, PictureSelection, PictureTagsWithSources} from '@/lib/types'

export async function listAllTags(): Promise<string[]> {
    const {data} = await apiClient.get<{ tags: string[] }>('/api/authenticated/tags')
    return data.tags
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
