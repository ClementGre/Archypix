import {apiClient} from './client'
import type {ServiceConfig, ServiceDetailResponse, ServiceResponse, ServiceType} from '@/lib/types'

const BASE = '/api/authenticated/tagging-services'

export async function listServices(): Promise<ServiceDetailResponse[]> {
    const {data} = await apiClient.get<ServiceDetailResponse[]>(BASE)
    return data
}

export async function getService(id: string): Promise<ServiceDetailResponse> {
    const {data} = await apiClient.get<ServiceDetailResponse>(`${BASE}/${id}`)
    return data
}

export async function createService(body: {
    service_type: ServiceType
    name?: string
    requires?: string[]
    excludes?: string[]
    config?: ServiceConfig
}): Promise<ServiceDetailResponse> {
    const {data} = await apiClient.post<ServiceDetailResponse>(BASE, body)
    return data
}

export async function updateService(
    id: string,
    body: { name?: string; enabled?: boolean; requires?: string[]; excludes?: string[] },
): Promise<ServiceResponse> {
    const {data} = await apiClient.patch<ServiceResponse>(`${BASE}/${id}`, body)
    return data
}

/** The single, uniform config-editing path for every service type (feature 20 §10.2). */
export async function replaceConfig(id: string, config: ServiceConfig): Promise<ServiceDetailResponse> {
    const {data} = await apiClient.put<ServiceDetailResponse>(`${BASE}/${id}/config`, {config})
    return data
}

export async function deleteService(id: string, promoteTags: boolean): Promise<void> {
    await apiClient.delete(`${BASE}/${id}`, {params: {promote_tags: promoteTags}})
}

/** Execution order of Rule + Segmentation services (mappings excluded — always first). */
export async function reorderServices(orderedIds: string[]): Promise<void> {
    await apiClient.post(`${BASE}/reorder`, {ordered_ids: orderedIds})
}
