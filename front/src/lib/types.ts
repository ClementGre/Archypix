// Shared domain types — wire shapes match API ref §10 and §6.3/6.6/6.8 exactly.

export type ExifSyncStatus = 'synced' | 'pending' | 'pending_job_creation' | 'unsupported'

export type PictureVariant = 'original' | 'small' | 'medium' | 'large'

export type SortField = 'captured_at' | 'ingested_at' | 'updated_at' | 'file_size' | 'filename'

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
    /** MIME type — distinguishes playable media (video/audio) from images in the grid. */
    mime_type: string | null
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
    /** The holder's own local soft-delete (trash); null when not trashed. */
    deleted_at: string | null
    /** Received only: the owner's soft-delete — drives the grace-window badge. */
    owner_deleted_at: string | null
    /** Received only: the owner's announced purge deadline. */
    owner_purge_at: string | null
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
    /** The holder's own local soft-delete (trash); null when not trashed. */
    deleted_at: string | null
    /** Received only: the owner's soft-delete — drives the grace-window badge. */
    owner_deleted_at: string | null
    /** Received only: the owner's announced purge deadline. */
    owner_purge_at: string | null
    /** Received only: the recipient's sticky per-field EXIF overrides (sparse FullExif). */
    local_exif_overrides: Record<string, unknown> | null
    /** Metadata-stripped content-dedup key (feature 11); null until hashed / for unstrippable formats. */
    content_hash: string | null
    /** Physical-copy provenance — the genuine original's owner identity (feature 11). Null when not a copy. */
    copy_source_owner_username: string | null
    copy_source_owner_instance: string | null
    copy_source_picture_id: string | null
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
    name: string
    message: string | null
    recipient_username: string
    recipient_instance: string
    status: ShareStatus
    allow_share_back: boolean
    /** Whether recipients may propose EXIF edits the owner auto-applies. */
    allow_exif_edit: boolean
    future: boolean
    /** ShareBack provenance: the incoming share (by its `outgoing_share_id`) this share answers. */
    shareback_of: string | null
    /** Announcement retry/backoff (set while errored/recovering). */
    last_error_at: string | null
    next_retry_at: string | null
    created_at: string
    /** When the share was closed (revoked or rejected); null while live. */
    revoked_at: string | null
}

export interface IncomingShareResponse {
    id: string
    sender_username: string
    sender_instance: string
    name: string
    message: string | null
    outgoing_share_id: string
    status: ShareStatus
    allow_share_back: boolean
    /** Propagated — whether the sender lets you propose EXIF edits the owner auto-applies. */
    allow_exif_edit: boolean
    /** Whether the sender auto-announces new pictures under the shared tag. */
    future: boolean
    /** Local `/SharedToMe/<sender>/…` tag (wire form) the received pictures land under. */
    shared_tag_path: string | null
    /** ISO timestamp of the sender's last picture announcement, or null if none yet. */
    last_announcement_received_at: string | null
    /** ShareBack provenance: the recipient's own outgoing share this is a share-back of. */
    shareback_of: string | null
    local_mapping_service_id: string | null
    /** When the incoming share was received. */
    created_at: string
    /** When the share was closed (revoked by sender or rejected here); null while live. */
    revoked_at: string | null
}

// ---------- Gallery filter object (camelCase; hooks map to query params) ----------

export interface PictureFilters {
    tag?: string | null
    /** Additional include tags (wire form) layered on top of `tag` via the sidebar menu. */
    include?: string[]
    /** Exclude tags (wire form). */
    exclude?: string[]
    /** Exact (strict, no-descendant) include tags (wire form). */
    exact?: string[]
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
/** Mirror behaviour for pictures below the `maxDepth` cut (feature 18 §7.2). */
export type DeeperMode = 'collapse' | 'exclude'
/** Per-node write-back tri-state (feature 18 §5). `null`/undefined = inherit. */
export type WriteBackEnabled = boolean | null

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
    /** Tri-state write-back override; `null`/undefined = inherit nearest explicit ancestor. */
    writeBackEnabled?: WriteBackEnabled
}

export interface MirrorNode extends NodeCommon {
    kind: 'mirror'
    tagRoot: string // ltree wire form
    keepDir?: boolean
    collapsed?: string[] // must be under tagRoot
    exclude?: string[] // may be foreign to tagRoot (pure picture-membership cut)
    maxDepth?: number // 0/absent = unrestricted; caps levels below tagRoot
    deeperMode?: DeeperMode // pictures below the cut (default 'collapse')
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

/** Write-only inbox: always shown, lists nothing, applies `onAdd` to every upload (feature 18 §4). */
export interface DropNode extends NodeCommon {
    kind: 'drop'
    name: string
    onAdd: WriteBackOp[]
}

export type HierarchyNode = MirrorNode | QueryNode | StaticNode | DropNode

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

/** WebDAV mount info from `GET/POST/PATCH /hierarchies/{id}/webdav`. */
export interface WebdavResponse {
    url: string
    token: string
    use_redirect: boolean
    enabled: boolean
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
    /** User-facing label (may be empty; UI falls back to a type label). */
    name: string
    service_type: ServiceType
    requires: string[]
    excludes: string[]
    enabled: boolean
    position: number
    created_at: string
    updated_at: string
}

export interface RuleTaggingRule {
    /** Server-assigned; absent on rules submitted to create/`PUT config`. */
    id: string
    /** Structured predicate tree (feature 13). See `lib/predicate.ts`. */
    predicate: RulePredicate
    assign_tag: string
}

/** GPS bounding box for a `gps_bbox` predicate node. */
export interface GpsBbox {
    lat_min: number
    lat_max: number
    lon_min: number
    lon_max: number
}

/** GPS centre + radius (km) for a `gps_radius` predicate node. */
export interface GpsRadius {
    lat: number
    lng: number
    km: number
}

/** A field-condition leaf: `{ field, <condition_key>: value, ... }`. */
export type FieldPredicate = { field: string } & Record<string, unknown>

/** A structured rule predicate tree (feature 13). */
export type RulePredicate =
    | { and: RulePredicate[] }
    | { or: RulePredicate[] }
    | { not: RulePredicate }
    | { gps_bbox: GpsBbox }
    | { gps_radius: GpsRadius }
    | FieldPredicate

// ---------- Calendar segmentation config (feature 20 §3) ----------

export type Hemisphere = 'north' | 'south'
export type PartBound = 'start' | 'end' | 'range'
export type PartCase = 'lower' | 'upper' | 'pascal'

/** Segmentation placeholders (feature 20 §4.1). */
export type SegmentationPlaceholder =
    | 'year'
    | 'iso_year'
    | 'quarter'
    | 'season'
    | 'month'
    | 'week'
    | 'day'
    | 'weekday'
    | 'daypart'

export interface PartFormat {
    numeric?: boolean
    pad?: number
    abbrev?: boolean
    case?: PartCase
    bound?: PartBound
    range_sep?: string
    inclusive_end?: boolean
}

export interface PartConfig {
    stride?: number
    format?: PartFormat
}

export interface SegmentationOffset {
    months?: number
    days?: number
    hours?: number
    minutes?: number
}

export interface SegmentationBand {
    /** 'YYYY-MM-DD' or null (−∞). Half-open `[from, to)`. */
    from: string | null
    /** 'YYYY-MM-DD' or null (+∞). */
    to: string | null
    enabled?: boolean
    template: string
    parts?: Record<string, PartConfig>
    offset?: SegmentationOffset
}

export interface CatchAll {
    /** Single ltree label ⇒ `root_tag.<name>`. */
    name: string
    include_undated: boolean
}

export interface SegmentationConfig {
    version: 1
    /** ltree wire-form root every band hangs under. */
    root_tag: string
    hemisphere?: Hemisphere
    catch_all: CatchAll | null
    /** Ordered; index 0 = highest precedence. */
    bands: SegmentationBand[]
}

// ---------- Service detail responses (tagged union on service_type) ----------

export interface SharedTagMappingServiceDetail extends ServiceBase {
    service_type: 'shared_tag_mapping'
    incoming_share_id: string
    assign_tags: string[]
    /** Derived server-side: the referenced incoming share is absent/inactive. */
    is_broken: boolean
}

export interface RuleServiceDetail extends ServiceBase {
    service_type: 'rule'
    rules: RuleTaggingRule[]
}

export interface SegmentationServiceDetail extends ServiceBase {
    service_type: 'segmentation'
    config: SegmentationConfig
}

export type ServiceDetailResponse =
    | SharedTagMappingServiceDetail
    | RuleServiceDetail
    | SegmentationServiceDetail

/** Type-specific config accepted by create / `PUT …/config` (feature 20 §10.2). */
export type RuleConfig = { rules: { id?: string; predicate: RulePredicate; assign_tag: string }[] }
export type SharedTagMappingConfig = { incoming_share_id: string; assign_tags: string[] }
export type ServiceConfig = RuleConfig | SegmentationConfig | SharedTagMappingConfig

export interface ServiceResponse {
    id: string
    name: string
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
    /** Days a trashed owned picture is kept before physical purge (default 30). */
    trash_retention_days: number
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

// ---------- Storage quota (feature 22) ----------

export type StorageWarnLevel = 'ok' | 'warn' | 'critical' | 'full'

export interface StorageBreakdown {
    originals_bytes: number
    originals_trashed_bytes: number
    versions_bytes: number
    versions_trashed_bytes: number
}

export interface StorageInfo {
    /** Quota in bytes; `null` = unlimited. */
    quota_bytes: number | null
    /** Billed total (originals + versions, live + trashed). */
    used_bytes: number
    /** Remaining bytes; `null` when unlimited. */
    available_bytes: number | null
    breakdown: StorageBreakdown
    /** Trashed originals + versions — the "empty trash to reclaim X" figure. */
    reclaimable_trash_bytes: number
    /** used / quota; `null` when unlimited. */
    usage_ratio: number | null
    warn_level: StorageWarnLevel
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
    /** Billed total (maintained counter, feature 22). */
    storage_bytes: number
    /** Quota in bytes; `null` = unlimited. */
    quota_bytes: number | null
    breakdown: StorageBreakdown
    /** storage_bytes / quota_bytes; `null` when unlimited. */
    usage_ratio: number | null
}

export interface UserStats {
    owned_picture_count: number
    received_picture_count: number
    storage_bytes: number
    quota_bytes: number | null
    originals_bytes: number
    originals_trashed_bytes: number
    versions_bytes: number
    versions_trashed_bytes: number
    usage_ratio: number | null
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

// ---------- Storage audit (feature 22 §8.3, admin) ----------

export interface PrefixUsage {
    object_count: number
    total_bytes: number
}

export interface StorageAuditResponse {
    buckets: {
        pictures: PrefixUsage
        versions: PrefixUsage
        thumbnails_small: PrefixUsage
        thumbnails_medium: PrefixUsage
        thumbnails_large: PrefixUsage
        staging: PrefixUsage
    }
    /** Free/untracked in the DB — the only place these bytes are visible. */
    thumbnails_bytes: number
    db_breakdown: StorageBreakdown
    db_billed_bytes: number
    /** Measured originals + versions. */
    s3_billed_bytes: number
    /** db_billed_bytes - s3_billed_bytes; nonzero -> drift to reconcile. */
    drift_bytes: number
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

/** Mode for the received-picture EXIF endpoint: a private local override, or a proposal to the owner. */
export type ExifEditMode = 'local' | 'propose'

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

/** Response of the recipient-local EXIF override endpoint (received pictures). */
export interface OverrideExifResponse {
    id: string
    captured_at: string | null
    gps_lat: number | null
    gps_lng: number | null
    gps_alt: number | null
    orientation: number | null
    exif_data: Record<string, unknown>
    local_exif_overrides: Record<string, unknown> | null
    updated_at: string
}

export interface TrashResponse {
    id: string
    deleted_at: string | null
}

// ---------- Batch editing (feature 14) ----------

/**
 * The homogenized picture filter (§3): the flat gallery or a hierarchy directory. Mirrors the
 * backend `PictureFilter` enum (tagged on `kind`). Scope/date params are shared by both kinds.
 */
export type PictureFilter =
    | {
    kind: 'flat'
    include_tags?: string[]
    exclude_tags?: string[]
    /** Exact (strict, no-descendant) include tags — feature 15 strict tag navigation. */
    exact?: string[]
    match?: 'all' | 'any'
    untagged?: boolean
    owned_only?: boolean
    shared_with_me?: boolean
    include_deleted?: boolean
    captured_after?: string
    captured_before?: string
}
    | {
    kind: 'hierarchy'
    hierarchy_id: string
    path: string
    owned_only?: boolean
    shared_with_me?: boolean
    include_deleted?: boolean
    captured_after?: string
    captured_before?: string
}

/**
 * The selection descriptor (§2). Effective set = `(resolve(query) ∪ include_ids) \ exclude_ids`,
 * always scoped server-side to the caller. `query == null` ⇒ pure explicit set.
 */
export interface PictureSelection {
    query?: PictureFilter | null
    include_ids?: string[]
    exclude_ids?: string[]
}

export type AggregateSection = 'summary' | 'tags' | 'exif'

export interface AggregateRequest {
    selection: PictureSelection
    sections?: AggregateSection[]
    tag_provenance?: boolean
}

export interface AggregateOwner {
    username: string
    instance: string
    count: number
}

export interface TagAggregate {
    path: string
    /** `count == summary.count` ⇒ on every selected picture; `< count` ⇒ on some. */
    count: number
    /** Pictures holding a *manual* row under this path — drives the remove affordance. */
    manual_count: number
    sources?: Array<{ source: TagSource; count: number }>
}

/** A type-aware per-field EXIF aggregate (§4.3). */
export type FieldAggregate =
    | {
    type: 'distinct'
    common: unknown | null
    distinct: Array<{ value: unknown; count: number }>
    distinct_overflow: number
    null_count: number
}
    | { type: 'numeric'; min: number | null; max: number | null; avg: number | null; null_count: number }
    | { type: 'date'; min: string | null; max: string | null; avg: string | null; null_count: number }
    | {
    type: 'gps'
    bbox: { lat_min: number; lat_max: number; lng_min: number; lng_max: number } | null
    centroid: { lat: number; lng: number } | null
    null_count: number
}

export interface AggregateResponse {
    count: number
    owned_count: number
    received_count: number
    total_file_size: number
    trashed_count: number
    owner_deleting_count: number
    thumbnail_pending_count: number
    duplicate_count: number
    owners: AggregateOwner[]
    exif_sync: Record<ExifSyncStatus, number>
    tags?: TagAggregate[]
    exif?: Record<string, FieldAggregate>
}

/** Batch EXIF apply mode (§6.1): edit locally, or propose to owners where the share grants it. */
export type BatchExifMode = 'local' | 'suggest'

/** Dry-run breakdown returned by every batch write when `dry_run: true` (§6.1). */
export interface BatchDryRun {
    affected: number
    // EXIF batch only:
    edited?: number
    suggested?: number
    local_override?: number
    unsupported?: number
    // tags batch only:
    added?: number
    removed?: number
}

/** Applied result of a batch EXIF edit. */
export interface BatchExifResult {
    affected: number
    edited: number
    suggested: number
    local_override: number
    unsupported: number
}

// ---------- Runtime settings (feature 23/24) ----------

export type SettingSource = 'default' | 'env' | 'db'

/** A single runtime-config field's metadata + value (backend/resolver `GET …/settings`). */
export interface FieldMeta {
    key: string
    /** The env var that would set (and lock) this field. */
    env: string
    /** UI grouping label. */
    group: string
    /** Current effective value (omitted for unset secrets). */
    value?: unknown
    is_set: boolean
    default_value?: unknown
    source: SettingSource
    /** `source === 'env'` — read-only, "defined by environment". */
    locked: boolean
    /** `false` = core/env-only field (never rendered as editable). */
    runtime_editable: boolean
    restart_required: boolean
    secret: boolean
    /** May be empty/None (an `Option<T>` setting). */
    nullable: boolean
    /** Rust-ish type tag: `string` | `bool` | `u16` | `i64` | `usize` | `f64` | `enum` | … */
    kind: string
    /** Present for enum kinds. */
    variants?: string[]
    /** The routine this field tunes, if any. */
    routine?: string
    description: string
    example: string
}

/** A background routine's live status + its tuning settings (`GET …/routines`). */
export interface RoutineInfo {
    name: string
    last_started_at: number | null
    last_finished_at: number | null
    last_error: string | null
    in_flight: number
    total_runs: number
    settings: FieldMeta[]
}

// ---------- Invites (feature 23 §6) ----------

export type RegistrationMode = 'open' | 'invite' | 'admin_invite'

export interface RegistrationInfo {
    mode: RegistrationMode
}

export interface Invite {
    code: string
    /** `null` = unlimited; `0` = tracking-only (open-mode referral). */
    max_uses: number | null
    uses: number
    expires_at: string | null
    created_by: string
    /** Resolver-only: the pinned backend the invitee joins. */
    instance_pin?: string | null
}

export interface InvitationGraph {
    invited_by: string | null
    invited: string[]
}

export interface InvitePreview {
    valid: boolean
    invited_by: string | null
}

// ---------- Resolver fleet admin (feature 24) ----------

export interface ResolverBackend {
    back_domain: string
    use_https: boolean
    delegation_expires_at: string | null
    user_count: number
    picture_count: number
    storage_bytes: number
    last_heartbeat_at: string | null
    healthy: boolean
    reachable: boolean
    accepting_registrations: boolean
    max_users: number | null
    version: string | null
    last_selected_at: string | null
    created_at: string
}

export interface ResolverOverview {
    total_users: number
    total_pictures: number
    total_storage_bytes: number
    backend_count: number
    reachable_count: number
    backends: ResolverBackend[]
}

export interface ResolverSession {
    session_token: string
    refresh_token: string
    expires_in_secs: number
}

/** One backend's outcome in a config-matrix fan-out PATCH. */
export interface ConfigMatrixPatchResult {
    ok: boolean
    status?: number
    error?: string
}
