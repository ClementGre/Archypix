import {apiClient} from './client'
import type {
    RulePredicate,
    RuleTaggingRule,
    SegmentationSegment,
    ServiceDetailResponse,
    ServiceResponse,
    ServiceType,
    SharedTagMappingRule,
} from '@/lib/types'

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
}): Promise<ServiceResponse> {
    const {data} = await apiClient.post<ServiceResponse>(BASE, body)
    return data
}

export async function updateService(
    id: string,
    body: { name?: string; enabled?: boolean; requires?: string[]; excludes?: string[] },
): Promise<ServiceResponse> {
    const {data} = await apiClient.patch<ServiceResponse>(`${BASE}/${id}`, body)
    return data
}

export async function deleteService(id: string, promoteTags: boolean): Promise<void> {
    await apiClient.delete(`${BASE}/${id}`, {params: {promote_tags: promoteTags}})
}

export async function reorderServices(orderedIds: string[]): Promise<void> {
    await apiClient.post(`${BASE}/reorder`, {ordered_ids: orderedIds})
}

// --- Shared-tag-mapping rules ---

export async function addMapping(
    serviceId: string,
    body: { incoming_share_id: string; assign_tag: string },
): Promise<SharedTagMappingRule> {
    const {data} = await apiClient.post<SharedTagMappingRule>(`${BASE}/${serviceId}/mappings`, body)
    return data
}

export async function deleteMapping(serviceId: string, ruleId: string): Promise<void> {
    await apiClient.delete(`${BASE}/${serviceId}/mappings/${ruleId}`)
}

// --- Rule-tagging rules ---

export async function addRule(
    serviceId: string,
    body: { predicate: RulePredicate; assign_tag: string },
): Promise<RuleTaggingRule> {
    const {data} = await apiClient.post<RuleTaggingRule>(`${BASE}/${serviceId}/rules`, body)
    return data
}

export async function updateRule(
    serviceId: string,
    ruleId: string,
    body: { predicate: RulePredicate; assign_tag: string },
): Promise<RuleTaggingRule> {
    const {data} = await apiClient.patch<RuleTaggingRule>(`${BASE}/${serviceId}/rules/${ruleId}`, body)
    return data
}

export async function reorderRules(serviceId: string, orderedIds: string[]): Promise<void> {
    await apiClient.post(`${BASE}/${serviceId}/rules/reorder`, {ordered_ids: orderedIds})
}

export async function deleteRule(serviceId: string, ruleId: string): Promise<void> {
    await apiClient.delete(`${BASE}/${serviceId}/rules/${ruleId}`)
}

// --- Segmentation segments ---

export async function addSegment(
    serviceId: string,
    body: { name: string; date_start: string; date_end: string; assign_tag: string; parent_segment_id?: string },
): Promise<SegmentationSegment> {
    const {data} = await apiClient.post<SegmentationSegment>(`${BASE}/${serviceId}/segments`, body)
    return data
}

export async function deleteSegment(serviceId: string, segmentId: string): Promise<void> {
    await apiClient.delete(`${BASE}/${serviceId}/segments/${segmentId}`)
}
