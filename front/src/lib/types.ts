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

// ---------- Tagging services ----------

export type ServiceType = 'shared_tag_mapping' | 'rule' | 'segmentation'

export interface ServiceBase {
    id: string
    service_type: ServiceType
    requires: string[]
    excludes: string[]
    enabled: boolean
    position: number
    created_at: string
    updated_at: string
}

export interface SharedTagMappingRule {
    id: string
    incoming_share_id: string
    assign_tag: string
    is_broken: boolean
}

export interface RuleTaggingRule {
    id: string
    predicate: string
    assign_tag: string
}

export interface SegmentationSegment {
    id: string
    name: string
    date_start: string
    date_end: string
    assign_tag: string
    parent_segment_id: string | null
}

export interface SharedTagMappingServiceDetail extends ServiceBase {
    service_type: 'shared_tag_mapping'
    mappings: SharedTagMappingRule[]
}

export interface RuleServiceDetail extends ServiceBase {
    service_type: 'rule'
    rules: RuleTaggingRule[]
}

export interface SegmentationServiceDetail extends ServiceBase {
    service_type: 'segmentation'
    segments: SegmentationSegment[]
}

export type ServiceDetailResponse =
    | SharedTagMappingServiceDetail
    | RuleServiceDetail
    | SegmentationServiceDetail

export interface ServiceResponse {
    id: string
    service_type: ServiceType
    requires: string[]
    excludes: string[]
    enabled: boolean
    position: number
    created_at: string
    updated_at: string
}

// ---------- User settings & profile ----------

export type VersioningMode = 'none' | 'original_copy' | 'full_versioning'

export interface UserSettings {
    user_id: string
    versioning_mode: VersioningMode
    created_at: string
    updated_at: string
}

export interface UserProfile {
    id: string
    username: string
    email: string
    display_name: string
    is_admin: boolean
}

// ---------- Jobs ----------

export type JobStatus = 'pending' | 'processing' | 'completed' | 'failed'

export interface Job {
    id: string
    owner_id: string
    job_type: string
    status: JobStatus
    result: Record<string, unknown> | null
    error_message: string | null
    retry_count: number
    max_retries: number
    picture_id: string | null
    created_at: string
    started_at: string | null
    completed_at: string | null
}

// ---------- EXIF editing ----------

export interface ExifOverrides {
    captured_at: string | null
    gps_lat: number | null
    gps_lng: number | null
    gps_alt: number | null
    orientation: number | null
    camera_brand: string | null
    camera_model: string | null
    focal_length_mm: number | null
    f_number: number | null
    iso_speed: number | null
    exposure_time_num: number | null
    exposure_time_den: number | null
}

export type ExifField = keyof ExifOverrides

export interface EditPictureResponse {
    id: string
    exif_sync_status: ExifSyncStatus
    captured_at: string | null
    gps_lat: number | null
    gps_lng: number | null
    gps_alt: number | null
    orientation: number | null
    exif_data: Record<string, unknown>
    updated_at: string
    job_id: string | null
}
