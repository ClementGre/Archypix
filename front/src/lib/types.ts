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
    orientation: number | null
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

// ---------- Hierarchies ----------

export type NamingStrategy = 'original' | 'date' | 'id'
export type SafeDeleteMode = 'singleBranch' | 'fullDelete'

export interface WriteBackOp {
    op: 'assign' | 'remove'
    path: string // ltree wire form
}

export interface WriteBack {
    onAdd: WriteBackOp[]
    onRemove: WriteBackOp[]
}

/** Fields shared by every hierarchy node, regardless of kind. */
export interface NodeCommon {
    id: string
    name?: string
    naming?: NamingStrategy | null
    safeDeleteMode?: SafeDeleteMode | null
}

export interface MirrorNode extends NodeCommon {
    kind: 'mirror'
    tagRoot: string // ltree wire form
    keepDir?: boolean
    collapsed?: string[]
    exclude?: string[]
}

export interface QueryNode extends NodeCommon {
    kind: 'query'
    name: string
    match?: 'all' | 'any'
    include?: string[]
    exclude?: string[]
    matchUntagged?: boolean
    writeBack?: WriteBack | null
    children?: HierarchyNode[]
}

export interface StaticNode extends NodeCommon {
    kind: 'static'
    name: string
    children?: HierarchyNode[]
}

export type HierarchyNode = MirrorNode | QueryNode | StaticNode

export type NodeKind = HierarchyNode['kind']

export interface HierarchyConfig {
    version: number
    safeDeleteMode: SafeDeleteMode
    naming: NamingStrategy
    writeBack: boolean
    nodes: HierarchyNode[]
}

/** List item shape from `GET /hierarchies`. */
export interface HierarchySummary {
    id: string
    name: string
    enabled: boolean
}

/** Full hierarchy from create/get/patch. */
export interface HierarchyDetail {
    id: string
    name: string
    enabled: boolean
    config: HierarchyConfig
    created_at: string
    updated_at: string
}

/** A directory in the resolved `tree` endpoint. */
export interface DirEntry {
    name: string
    writable: boolean
    child_count: number
    picture_count: number | null
    children?: DirEntry[]
}

export interface HierarchyTreeResponse {
    path: string
    directories: DirEntry[]
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

// ---------- Admin ----------

export type JobType = 'gen_thumbnail' | 'ml_style' | 'ml_people' | 'ml_group_location' | 'edit_picture'

export interface InstanceHealth {
    global_domain: string
    back_domain: string
    db_connected: boolean
    redis_connected: boolean
    last_worker_activity_at: string | null
}

export interface InstanceStats {
    user_count: number
    owned_picture_count: number
    received_picture_count: number
    total_storage_bytes: number
    job_counts: {
        pending: number
        processing: number
        completed: number
        failed: number
    }
    errored_share_count: number
    pending_first_announcement_count: number
    dirty_picture_count: number
    last_worker_activity_at: string | null
}

export interface ConsistencyCheck {
    stuck_exif_pending_count: number
    pictures_without_thumbnail_count: number
    broken_mapping_count: number
}

export interface AdminUserResponse {
    id: string
    username: string
    email: string
    display_name: string
    is_admin: boolean
    storage_bytes: number
}

export interface UserStats {
    owned_picture_count: number
    received_picture_count: number
    storage_bytes: number
    job_counts: {
        pending: number
        processing: number
        completed: number
        failed: number
    }
    outgoing_share_counts: Record<ShareStatus, number>
    incoming_share_counts: Record<ShareStatus, number>
    dirty_picture_count: number
    errored_share_count: number
}

export interface OutgoingShareRow {
    id: string
    owner_id: string
    tag_path: string
    recipient_username: string
    recipient_instance: string
    allow_share_back: boolean
    future: boolean
    status: ShareStatus
    created_at: string
    revoked_at: string | null
}

export interface IncomingShareRow {
    id: string
    recipient_id: string
    sender_username: string
    sender_instance: string
    outgoing_share_id: string
    local_mapping_service_id: string | null
    status: ShareStatus
    allow_share_back: boolean
    created_at: string
    revoked_at: string | null
}

export interface UserSharesResponse {
    outgoing: OutgoingShareRow[]
    incoming: IncomingShareRow[]
}

export interface AdminJobResponse {
    id: string
    owner_id: string
    owner_username: string
    job_type: JobType
    status: JobStatus
    retry_count: number
    max_retries: number
    error_message: string | null
    picture_id: string | null
    claimed_by: string | null
    created_at: string
    started_at: string | null
    completed_at: string | null
}

export interface ErroredShareResponse {
    id: string
    owner_id: string
    owner_username: string
    tag_path: string
    recipient_username: string
    recipient_instance: string
    next_retry_at: string | null
    last_error_at: string | null
    created_at: string
}

export interface FederationInstanceResponse {
    instance: string
    outgoing_share_count: number
    incoming_share_count: number
    errored_share_count: number
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
