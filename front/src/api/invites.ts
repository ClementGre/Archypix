import axios from 'axios'
import {apiClient} from '@/api/client'
import {GLOBAL_DOMAIN, originFor} from '@/lib/constants'
import type {InvitationGraph, Invite, InvitePreview, RegistrationInfo} from '@/lib/types'

/**
 * Invites (feature 23 §6). The user-facing surface is the same on a standalone backend and in resolver
 * mode — the backend transparently forwards to the resolver when `USE_RESOLVER` is set.
 */
export async function listInvites(): Promise<Invite[]> {
    const {data} = await apiClient.get<Invite[]>('/api/authenticated/invites')
    return data
}

export async function mintInvite(body: {
    max_uses: number | null
    expires_in_days: number | null
}): Promise<Invite> {
    const {data} = await apiClient.post<Invite>('/api/authenticated/invites', body)
    return data
}

export async function revokeInvite(code: string): Promise<void> {
    await apiClient.delete(`/api/authenticated/invites/${encodeURIComponent(code)}`)
}

export async function getInvitations(): Promise<InvitationGraph> {
    const {data} = await apiClient.get<InvitationGraph>('/api/authenticated/me/invitations')
    return data
}

/**
 * Unauthenticated preview of an invite code (register page). Served at the same path by a standalone
 * backend and the resolver, so the register flow doesn't need to know the topology.
 */
export async function previewInvite(code: string, domain: string = GLOBAL_DOMAIN): Promise<InvitePreview> {
    const {data} = await axios.get<InvitePreview>(`${originFor(domain)}/api/public/invites/${encodeURIComponent(code)}`)
    return data
}

/** Effective registration mode where signups land — lets the mint UI adapt (tracking vs. gated). */
export async function getRegistrationInfo(domain: string = GLOBAL_DOMAIN): Promise<RegistrationInfo> {
    const {data} = await axios.get<RegistrationInfo>(`${originFor(domain)}/api/public/registration-info`)
    return data
}
