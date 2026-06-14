import {apiClient} from './client'
import type {PictureTagsWithSources} from '@/lib/types'

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

export async function batchEditTags(body: {
    picture_ids: string[]
    add_tags?: string[]
    remove_tags?: string[]
}): Promise<void> {
    await apiClient.patch('/api/authenticated/tags', body)
}
