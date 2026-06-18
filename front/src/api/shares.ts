import {apiClient} from './client'
import type {IncomingShareResponse, ShareResponse} from '@/lib/types'

export async function listOutgoingShares(): Promise<ShareResponse[]> {
    const {data} = await apiClient.get<ShareResponse[]>('/api/authenticated/shares/outgoing')
    return data
}

export async function listIncomingShares(): Promise<IncomingShareResponse[]> {
    const {data} = await apiClient.get<IncomingShareResponse[]>('/api/authenticated/shares/incoming')
    return data
}

export async function acceptIncomingShare(id: string): Promise<void> {
    await apiClient.post(`/api/authenticated/shares/incoming/${id}/accept`)
}

export async function rejectIncomingShare(id: string): Promise<void> {
    await apiClient.post(`/api/authenticated/shares/incoming/${id}/reject`)
}

export async function revokeOutgoingShare(id: string): Promise<void> {
    await apiClient.post(`/api/authenticated/shares/outgoing/${id}/revoke`)
}

export interface CreateOutgoingShareBody {
    tag_path: string
    name: string
    message?: string
    recipient_username: string
    recipient_instance: string
    allow_share_back?: boolean
    future?: boolean
    shareback_of?: string
}

export async function createOutgoingShare(body: CreateOutgoingShareBody): Promise<ShareResponse> {
    const {data} = await apiClient.post<ShareResponse>('/api/authenticated/shares/outgoing', body)
    return data
}
