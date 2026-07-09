import axios from 'axios'
import {apiClient} from '@/api/client'
import {getResolverInfo} from '@/api/resolve'
import {GLOBAL_DOMAIN} from '@/lib/constants'
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
 * Unauthenticated preview of an invite code (register page). Bootstraps the domain
 * (`/archypix-resolver/info`) to find where the public surface lives, then hits `{api_url}/api/public/…`
 * — served directly by a standalone backend and under the `/archypix-resolver` prefix by a resolver.
 */
export async function previewInvite(code: string, domain: string = GLOBAL_DOMAIN): Promise<InvitePreview> {
    const {api_url} = await getResolverInfo(domain)
    const {data} = await axios.get<InvitePreview>(`${api_url}/api/public/invites/${encodeURIComponent(code)}`)
    return data
}

/** Effective registration mode where signups land — lets the mint UI adapt (tracking vs. gated). */
export async function getRegistrationInfo(domain: string = GLOBAL_DOMAIN): Promise<RegistrationInfo> {
    const {api_url} = await getResolverInfo(domain)
    const {data} = await axios.get<RegistrationInfo>(`${api_url}/api/public/registration-info`)
    return data
}
