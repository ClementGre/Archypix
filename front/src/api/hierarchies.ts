import {apiClient} from './client'
import type {
    HierarchyConfig,
    HierarchyDetail,
    HierarchySummary,
    HierarchyTreeResponse,
    PictureListResponse,
    PictureVariant,
    TrashFilter,
    WebdavResponse,
} from '@/lib/types'

const BASE = '/api/authenticated/hierarchies'

// --- CRUD ---

export async function listHierarchies(): Promise<HierarchySummary[]> {
    const {data} = await apiClient.get<HierarchySummary[]>(BASE)
    return data
}

export async function getHierarchy(id: string): Promise<HierarchyDetail> {
    const {data} = await apiClient.get<HierarchyDetail>(`${BASE}/${id}`)
    return data
}

export async function createHierarchy(body: {
    name: string
    config?: HierarchyConfig
}): Promise<HierarchyDetail> {
    const {data} = await apiClient.post<HierarchyDetail>(BASE, body)
    return data
}

export async function updateHierarchy(
    id: string,
    body: { name?: string; enabled?: boolean; config?: HierarchyConfig },
): Promise<HierarchyDetail> {
    const {data} = await apiClient.patch<HierarchyDetail>(`${BASE}/${id}`, body)
    return data
}

export async function deleteHierarchy(id: string): Promise<void> {
    await apiClient.delete(`${BASE}/${id}`)
}

// --- WebDAV mount (token management) ---

export async function getWebdav(id: string): Promise<WebdavResponse> {
    const {data} = await apiClient.get<WebdavResponse>(`${BASE}/${id}/webdav`)
    return data
}

export async function regenerateWebdavToken(id: string): Promise<WebdavResponse> {
    const {data} = await apiClient.post<WebdavResponse>(`${BASE}/${id}/webdav/regenerate`)
    return data
}

export async function setWebdavUseRedirect(
    id: string,
    use_redirect: boolean,
): Promise<WebdavResponse> {
    const {data} = await apiClient.patch<WebdavResponse>(`${BASE}/${id}/webdav`, {use_redirect})
    return data
}

// --- Navigation (read resolver) ---

export interface TreeParams {
    path?: string
    depth?: number
    counts?: boolean
}

export async function getHierarchyTree(
    id: string,
    params: TreeParams,
): Promise<HierarchyTreeResponse> {
    const query: Record<string, unknown> = {}
    if (params.path) query.path = params.path
    if (params.depth != null) query.depth = params.depth
    if (params.counts) query.counts = true
    const {data} = await apiClient.get<HierarchyTreeResponse>(`${BASE}/${id}/tree`, {params: query})
    return data
}

export interface BrowseParams {
    path?: string
    page: number
    page_size: number
    sort?: string
    order?: string
    trash?: TrashFilter
    owned_only?: boolean
    shared_with_me?: boolean
    captured_after?: string
    captured_before?: string
    thumbnail?: PictureVariant
}

export async function browseHierarchy(
    id: string,
    params: BrowseParams,
): Promise<PictureListResponse> {
    const query: Record<string, unknown> = {}
    for (const [k, v] of Object.entries(params)) {
        if (v !== undefined) query[k] = v
    }
    const {data} = await apiClient.get<PictureListResponse>(`${BASE}/${id}/browse`, {params: query})
    return data
}
