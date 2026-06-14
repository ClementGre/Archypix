// Shared domain types — wire shapes match API ref §10 and §6.3/6.6/6.8 exactly.

export type ExifSyncStatus = 'synced' | 'pending' | 'unsupported'

export type PictureVariant = 'original' | 'small' | 'medium' | 'large'

export type SortField = 'captured_at' | 'ingested_at' | 'updated_at'

export type SortOrder = 'asc' | 'desc'

export type ShareStatus =
    | 'pending'
    | 'pending_first_announcement'
    | 'active'
    | 'errored'
    | 'revoked'
    | 'tombstoned'

export type TagSource = 'manual' | 'rule' | 'segment' | 'share_mapping' | 'incoming_share'

// ---------- Pictures ----------

export interface PictureListItem {
    id: string
    filename: string | null
    width: number | null
    height: number | null
    captured_at: string | null
    ingested_at: string
    blurhash: string | null
    thumbnail_url: string | null
    owned: boolean
    owner_username: string | null
    owner_instance: string | null
    exif_sync_status: ExifSyncStatus
}

export interface PictureListResponse {
    total: number
    page: number
    page_size: number
    items: PictureListItem[]
}

export interface PictureVersion {
    id: string
    picture_id: string
    version_number: number
    file_size: number | null
    mime_type: string | null
    created_at: string
}

export interface PictureDetail {
    id: string
    filename: string | null
    mime_type: string | null
    file_size: number | null
    width: number | null
    height: number | null
    captured_at: string | null
    ingested_at: string
    updated_at: string
    gps_lat: number | null
    gps_lng: number | null
    gps_alt: number | null
    orientation: number | null
    exif_data: Record<string, unknown>
    exif_sync_status: ExifSyncStatus
    owner_username: string | null
    owner_instance_domain: string | null
    versions: PictureVersion[]
}

// ---------- Tags ----------

export interface PictureTagsWithSources {
    tags: Array<{
        path: string
        sources: Array<{
            source: TagSource
            source_id: string | null
        }>
    }>
}

// ---------- Shares ----------

export interface ShareResponse {
    id: string
    tag_path: string
    recipient_username: string
    recipient_instance: string
    status: ShareStatus
    allow_share_back: boolean
    future: boolean
}

export interface IncomingShareResponse {
    id: string
    sender_username: string
    sender_instance: string
    outgoing_share_id: string
    status: ShareStatus
    allow_share_back: boolean
    local_mapping_service_id: string | null
}

// ---------- Gallery filter object (camelCase; hooks map to query params) ----------

export interface PictureFilters {
    tag?: string | null
    scope?: 'all' | 'owned' | 'shared'
    includeDeleted?: boolean
    sort?: SortField
    order?: SortOrder
    capturedAfter?: string | null
    capturedBefore?: string | null
}
