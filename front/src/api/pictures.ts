import {apiClient} from './client'
import type {EditPictureResponse, ExifField, ExifOverrides, Job, PictureDetail, PictureListResponse, PictureVariant} from '@/lib/types'

export interface ListPicturesParams {
    page: number
    page_size: number
    sort?: string
    order?: string
    tag?: string
    owned_only?: boolean
    shared_with_me?: boolean
    include_deleted?: boolean
    captured_after?: string
    captured_before?: string
    thumbnail?: PictureVariant
}

export async function listPictures(params: ListPicturesParams): Promise<PictureListResponse> {
    // Build params object omitting undefined values so axios doesn't send empty keys.
    const query: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(params)) {
        if (v !== undefined) query[k] = v
    }
    const {data} = await apiClient.get<PictureListResponse>('/api/authenticated/pictures', {
        params: query,
    })
    return data
}

export async function getPicture(id: string): Promise<PictureDetail> {
    const {data} = await apiClient.get<PictureDetail>(`/api/authenticated/pictures/${id}`)
    return data
}

export async function getPictureUrl(
    id: string,
    variant: PictureVariant,
): Promise<{ url: string; variant: PictureVariant }> {
    const {data} = await apiClient.get<{ url: string; variant: PictureVariant }>(
        `/api/authenticated/pictures/${id}/url`,
        {params: {variant}},
    )
    return data
}

export async function editPicture(
    id: string,
    body: { set?: Partial<ExifOverrides>; clear?: ExifField[] },
): Promise<EditPictureResponse> {
    const {data} = await apiClient.post<EditPictureResponse>(
        `/api/authenticated/pictures/${id}/edit`,
        body,
    )
    return data
}

export async function getJob(id: string): Promise<Job> {
    const {data} = await apiClient.get<Job>(`/api/authenticated/jobs/${id}`)
    return data
}
