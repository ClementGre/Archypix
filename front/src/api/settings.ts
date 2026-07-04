import {apiClient} from '@/api/client'
import type {StorageInfo, UserProfile, UserSettings, VersioningMode} from '@/lib/types'

export async function getSettings(): Promise<UserSettings> {
    const {data} = await apiClient.get<UserSettings>('/api/authenticated/settings')
    return data
}

/** The caller's storage quota, usage, and breakdown (feature 22). */
export async function getStorage(): Promise<StorageInfo> {
    const {data} = await apiClient.get<StorageInfo>('/api/authenticated/me/storage')
    return data
}

export async function updateSettings(body: {
    versioning_mode?: VersioningMode
    trash_retention_days?: number
}): Promise<UserSettings> {
    const {data} = await apiClient.patch<UserSettings>('/api/authenticated/settings', body)
    return data
}

export async function updateProfile(body: { display_name?: string; email?: string }): Promise<UserProfile> {
    const {data} = await apiClient.patch<UserProfile>('/api/authenticated/users/me', body)
    return data
}
