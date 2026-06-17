# API Reference

Complete reference for all Archypix HTTP endpoints. Intended as the primary source of truth for frontend development — frontend agents and developers
should read this file rather than the Rust source code.

---

## 1. Overview

### Base URL

The backend exposes all routes from a single base URL. In local development, Vite proxies `/api/*` to `http://localhost:3000`. In production,
`VITE_API_BASE_URL` points to the resolved backend domain.

### Route groups

| Prefix                   | Auth type                   | Notes                                                                |
|--------------------------|-----------------------------|----------------------------------------------------------------------|
| `/api/auth/*`            | None / User JWT             | Login, refresh, logout, me                                           |
| `/api/public/*`          | None                        | Registration (standalone mode), public profiles                      |
| `/api/authenticated/*`   | User JWT                    | All regular user actions                                             |
| `/api/admin/*`           | User JWT + `is_admin=true`  | Admin panel                                                          |
| `/api/worker/*`          | Worker JWT                  | Worker-facing only, not called by frontend                           |
| `/api/federation/*`      | Federation JWT              | Server-to-server only, not called by frontend                        |
| `/api/resolver/*`        | Resolver JWT                | Resolver-facing only, not called by frontend                         |
| `/.well-known/webfinger` | None                        | Identity resolution                                                  |
| `/webdav/{slug}/*`       | Per-hierarchy token (Basic) | WebDAV mount of a hierarchy; external sync clients, not the frontend |

---

## 2. Authentication

### Token types

The backend issues **user JWTs** for authenticated sessions. The frontend only ever uses user JWTs.

```ts
interface JwtClaims {
    sub: string;        // username
    uid: string;        // user UUID
    is_admin: boolean;
    instance: string;   // global domain of the issuing instance
    token_type: "user" | "resolver" | "federation" | "worker";
    aud: string;        // backend domain of the verifying instance
    iss: string;        // backend domain of the signing instance
    exp: number;        // Unix seconds
    iat: number;
    jti: string;        // unique token ID
}
```

### Using tokens

Attach the access token to every authenticated request:

```
Authorization: Bearer <access_token>
```

### Refresh flow

The backend issues both an `access_token` (short-lived) and a `refresh_token` (longer-lived) on login. The Axios interceptor in `src/api/client.ts`
automatically:

1. Attaches `Authorization` header on every request.
2. On 401, calls `POST /api/auth/refresh` once, updates the stored token, and retries the original request.
3. If refresh also fails, clears auth state and redirects to `/login`.

Both tokens are stored in `localStorage`. Session invalidation happens on logout.

---

## 3. Wire Format Conventions

### Tag paths

Tag paths are stored and transmitted in **ltree dot-separated form** on the wire:

| Display form                      | Wire form                               |
|-----------------------------------|-----------------------------------------|
| `/Photos/Travel/Alps`             | `Photos.Travel.Alps`                    |
| `/SharedToMe/alice@ex.com/Photos` | `SharedToMe.alice_AT_ex_DOT_com.Photos` |

The `TagPath` helper in `src/lib/utils.ts` converts between the two forms. All tag-related request fields (filters, `add_tags`, `remove_tags`,
`assign_tag`, `requires`, `excludes`) and all response tag arrays use dot-separated ltree form.

Label characters are restricted to `[A-Za-z0-9_]`. The `/` separator becomes `.` on the wire.

### Protected tag prefixes

The `SharedToMe` prefix is reserved by the system. User-facing tag inputs must **not** allow it (the API validates this). Use
`allow_protected = false` in the frontend `TagPath` helper when accepting manual user input.

### Datetimes

All datetimes are ISO 8601 / RFC3339 strings in UTC unless noted otherwise. EXIF `captured_at` from the worker may arrive in `YYYY:MM:DD HH:MM:SS`
format — the backend normalizes it.

### UUIDs

All IDs are UUID v4 strings.

---

## 4. Auth Endpoints

### `POST /api/auth/login`

**Auth:** None

**Request:**

```ts
{
    username: string;
    password: string;
}
```

**Response `200`:**

```ts
{
    access_token: string;
    refresh_token: string;
}
```

---

### `POST /api/auth/refresh`

**Auth:** None

**Request:**

```ts
{
    refresh_token: string;
}
```

**Response `200`:**

```ts
{
    access_token: string;
    refresh_token: string;
}
```

---

### `POST /api/auth/logout`

**Auth:** User JWT

**Request:**

```ts
{
    refresh_token ? : string;  // if provided, also invalidates that refresh token
}
```

**Response `200`:**

```ts
{
    logged_out: true
}
```

---

### `GET /api/auth/me`

**Auth:** User JWT

**Response `200`:**

```ts
{
    id: string;            // UUID
    username: string;
    email: string;
    display_name: string;
    is_admin: boolean;
}
```

---

## 5. Public Endpoints

### `POST /api/public/users`

Register a new user. **Only available when `USE_RESOLVER=false`** (standalone mode). Returns 400 when the resolver is active — registration goes
through `POST /api/register` on the resolver service instead.

**Auth:** None

**Request:**

```ts
{
    username: string;
    email: string;
    display_name: string;
    password: string;
}
```

**Response `200`:**

```ts
{
    id: string;
    username: string;
    email: string;
    display_name: string;
}
```

---

### `GET /api/public/users/{username}`

Get a user's public profile.

**Auth:** None

**Path params:** `username: string`

**Response `200`:**

```ts
{
    id: string;
    username: string;
    email: string;
    display_name: string;
}
```

**Errors:** 404 if not found.

---

## 6. Authenticated User Endpoints

All endpoints in this section require `Authorization: Bearer <user_jwt>`.

---

### 6.1 Profile

#### `PATCH /api/authenticated/users/me`

Update the current user's profile.

**Request:**

```ts
{
    display_name ? : string;
    email ? : string;
}
```

**Response `200`:**

```ts
{
    id: string;
    username: string;
    email: string;
    display_name: string;
}
```

---

#### `GET /api/authenticated/settings`

Get the current user's settings.

**Response `200`:**

```ts
{
    user_id: string;
    versioning_mode: VersioningMode;
    created_at: string;
    updated_at: string;
}
```

---

#### `PATCH /api/authenticated/settings`

Update settings.

**Request:**

```ts
{
    versioning_mode: VersioningMode;
}
```

**Response `200`:** Same shape as GET settings.

---

### 6.2 Pictures — Upload

Upload is a three-step process for each file: begin (get a presigned S3 URL), PUT directly to S3, then complete. For batch uploads, step 1 can be
combined for all files in a single call.

#### Step 1a (single): `POST /api/authenticated/pictures/uploads`

**Request:**

```ts
{
    filename: string;  // must be non-empty
}
```

**Response `200`:**

```ts
{
    picture_id: string;   // UUID — use in step 3
    presigned_url: string; // PUT the file bytes here directly
}
```

#### Step 1b (batch): `POST /api/authenticated/pictures/uploads/batch`

Presigns multiple upload slots in one round-trip. Returns results in the same order as the input array. Capped at 100 filenames per call.

**Request:**

```ts
{
    filenames: string[];  // 1–100 non-empty filenames
}
```

**Response `200`:** `Array<{ picture_id: string; presigned_url: string }>`

Use this for multi-file uploads to avoid N serial requests before any S3 PUT can begin. The complete step (step 3) is still called individually per
file as each S3 upload finishes — do not wait for all files to finish before completing any.

#### Step 2: PUT the file

The client PUTs the raw file bytes to `presigned_url`. No auth header needed (presigned URL has embedded credentials). Include `Content-Type` matching
the file's MIME type.

#### Step 3: `POST /api/authenticated/pictures/uploads/{id}/complete`

**Path params:** `id: string` — the `picture_id` from step 1.

**Request:**

```ts
{
    mime_type ? : string;
    file_size ? : number;      // bytes (i64)
    width ? : number;          // pixels (i32)
    height ? : number;
    exif_data ? : object;      // arbitrary EXIF key-value pairs
    captured_at ? : string;    // ISO 8601 datetime
    initial_tags ? : string[]; // ltree wire-form paths — assigned as manual tags atomically with picture creation
}
```

All fields are optional — the backend fills in EXIF fields from worker extraction if omitted. `initial_tags` paths must not start with `SharedToMe`.

**Response `200`:**

```ts
{
    id: string
}  // picture UUID
```

**Side-effects:** creates the picture row, assigns any `initial_tags` as `manual` source tags, enqueues a `gen_thumbnail` job (EXIF extraction +
thumbnail generation), and wakes the pipeline. All of these happen atomically in a single DB transaction.

---

### 6.3 Pictures — List & Details

#### `GET /api/authenticated/pictures`

Paginated picture list.

**Query params:**
| Name | Type | Default | Description |
|---|---|---|---|
| `page` | `number` | `1` | Page number (1-indexed) |
| `page_size` | `number` | `50` | Items per page |
| `sort` | `"captured_at" \| "ingested_at" \| "updated_at"` | `"ingested_at"` | Sort field |
| `order` | `"asc" \| "desc"` | `"desc"` | Sort direction |
| `tag` | `string` | — | Filter by a single ltree tag path (dot-separated); convenience alias for one `include_tags` entry |
| `include_tags` | `string` | — | Comma-separated ltree paths the picture must match (inclusive `<@`), combined per `match` |
| `exclude_tags` | `string` | — | Comma-separated ltree paths; reject the picture if it has any (inclusive) |
| `match` | `"all" \| "any"` | `"all"` | Combinator over `include_tags` (`all` = AND, `any` = OR) |
| `untagged` | `boolean` | `false` | Only pictures with no stored tag of any source. Mutually exclusive with `include_tags`/`exclude_tags` |
| `owned_only` | `boolean` | `false` | Only show pictures owned by this user |
| `shared_with_me` | `boolean` | `false` | Only show pictures received via incoming shares |
| `include_deleted` | `boolean` | `false` | Include soft-deleted pictures (trash view) |
| `captured_after` | `string` | — | ISO 8601 datetime — lower bound on capture date |
| `captured_before` | `string` | — | ISO 8601 datetime — upper bound on capture date |
| `thumbnail` | `"original" \| "small" \| "medium" \| "large"` | — | If set, each item includes a `thumbnail_url` presigned for this variant |

**Response `200`:**

```ts
{
    total: number;       // i64
    page: number;
    page_size: number;
    items: PictureListItem[];
}

interface PictureListItem {
    id: string;
    filename: string | null;
    width: number | null;
    height: number | null;
    captured_at: string | null;
    ingested_at: string;
    blurhash: string | null;
    orientation: number | null;    // EXIF orientation (1–8); thumbnails are raw pixels — the client rotates them
    thumbnail_url: string | null;  // only when thumbnail query param is set
    owned: boolean;                // false for received (shared-to-me) pictures
    owner_username: string | null; // set when owned=false
    owner_instance: string | null; // global domain of the owning instance
    exif_sync_status: ExifSyncStatus;
}
```

---

#### `GET /api/authenticated/pictures/{id}`

Full picture details including version history.

**Path params:** `id: string` — UUID

**Response `200`:**

```ts
{
    id: string;
    filename: string | null;
    mime_type: string | null;
    file_size: number | null;      // bytes (i64)
    width: number | null;
    height: number | null;
    captured_at: string | null;
    ingested_at: string;
    updated_at: string;
    gps_lat: number | null;        // f64
    gps_lng: number | null;
    gps_alt: number | null;        // metres (i32)
    orientation: number | null;    // EXIF orientation value (i16), 1–8
    exif_data: object;             // arbitrary EXIF fields (camera make/model, focal length, etc.)
    exif_sync_status: ExifSyncStatus;
    owner_username: string | null;
    owner_instance_domain: string | null;
    versions: PictureVersion[];
}

interface PictureVersion {
    id: string;
    picture_id: string;
    version_number: number;
    file_size: number | null;
    mime_type: string | null;
    created_at: string;
}
```

**Errors:** 404 if picture not found or not accessible by the current user.

---

#### `GET /api/authenticated/pictures/{id}/url`

Get a presigned download URL for a picture variant.

**Path params:** `id: string`

**Query params:**
| Name | Type | Required | Description |
|---|---|---|---|
| `variant` | `PictureVariant` | yes | Which variant to fetch |

`PictureVariant`: `"original" | "small" | "medium" | "large"`

- `small` — WebP thumbnail, ~100px height
- `medium` — WebP thumbnail, ~500px height
- `large` — WebP thumbnail, ~1000px height
- `original` — original uploaded file at full resolution

**Response `200`:**

```ts
{
    url: string;
    variant: PictureVariant;
}
```

The URL is a presigned S3 URL valid for a limited time (~15 minutes). Cache it; do not request a new URL per render. The `PhotoGrid` component should
batch-presign via the `thumbnail` query param on the list endpoint instead.

**Important for received pictures:** the presigned URL may point to a different backend (the original owner's S3). The frontend must follow redirects
and not assume the URL is on the current backend domain.

---

### 6.4 Pictures — EXIF Editing

EXIF edits are **write-through**: the DB is updated synchronously at request time, and a background `edit_picture` job reconciles the embedded EXIF in
the S3 file. The `exif_sync_status` field tracks whether the file is up-to-date.

#### `POST /api/authenticated/pictures/{id}/edit`

Edit a single picture's EXIF.

**Path params:** `id: string` — must be an owned picture.

**Request:**

```ts
{
    set ? : Partial<ExifOverrides>;   // fields to set (null values in `set` are ignored — use `clear`)
    clear ? : ExifField[];            // fields to explicitly null out
}

interface ExifOverrides {
    captured_at: string | null;
    gps_lat: number | null;
    gps_lng: number | null;
    gps_alt: number | null;
    orientation: number | null;
    camera_brand: string | null;
    camera_model: string | null;
    focal_length_mm: number | null;
    f_number: number | null;
    iso_speed: number | null;
    exposure_time_num: number | null;
    exposure_time_den: number | null;
}

type ExifField = "captured_at" | "gps_lat" | "gps_lng" | "gps_alt" | "orientation" |
    "camera_brand" | "camera_model" | "focal_length_mm" | "f_number" |
    "iso_speed" | "exposure_time_num" | "exposure_time_den";
```

**Response `200`:**

```ts
{
    id: string;
    exif_sync_status: ExifSyncStatus;
    captured_at: string | null;
    gps_lat: number | null;
    gps_lng: number | null;
    gps_alt: number | null;
    orientation: number | null;
    exif_data: object;
    updated_at: string;
    job_id: string | null;  // null if format is "unsupported" (no file reconcile needed)
}
```

When `exif_sync_status = "unsupported"`, the format cannot embed EXIF (e.g. PNG). The DB is still updated but no job is enqueued.

---

#### `PATCH /api/authenticated/pictures/exif`

Batch EXIF edit on multiple owned pictures.

**Request:**

```ts
{
    picture_ids: string[];         // must all be owned by the current user
    set ? : Partial<ExifOverrides>;
    clear ? : ExifField[];
}
```

**Response `200`:**

```ts
{
    updated: number;       // count of pictures updated in DB
    jobs: string[];        // job UUIDs created (one per picture that needs file reconcile)
    unsupported: number;   // count of pictures whose format cannot embed EXIF
}
```

---

#### `POST /api/authenticated/pictures/{id}/exif/resync`

Re-enqueue a stuck EXIF sync (picture stuck in `exif_sync_status = "pending"` with no active job).

**Path params:** `id: string`

**Response `200`:** Full `Job` object (the newly enqueued job).

---

### 6.5 Pictures — Jobs

#### `GET /api/authenticated/pictures/{id}/jobs`

List all processing jobs for a picture.

**Path params:** `id: string`

**Response `200`:** `Job[]`

---

#### `GET /api/authenticated/jobs/{id}`

Get the status of a specific job.

**Path params:** `id: string`

**Response `200`:**

```ts
interface Job {
    id: string;
    owner_id: string;
    job_type: JobType;
    status: JobStatus;
    config: object;         // JobConfig, job-type-specific
    result: object | null;  // set on completion
    error_message: string | null;
    retry_count: number;
    max_retries: number;
    idempotency_key: string | null;
    picture_id: string | null;
    claimed_by: string | null;
    claim_token: string | null;
    created_at: string;
    started_at: string | null;
    completed_at: string | null;
}
```

**Errors:** 404 if not found or job belongs to a different user.

---

### 6.6 Tags

#### `GET /api/authenticated/tags`

List tags. Behavior varies by query params.

**Query params:**
| Name | Type | Default | Description |
|---|---|---|---|
| `picture_id` | `string` | — | When set, returns tags for that specific picture only |
| `with_sources` | `boolean` | `false` | When true (and `picture_id` is set), returns per-source provenance |

**Response `200` — all user tags (no `picture_id`):**

```ts
{
    tags: string[];  // all distinct ltree paths the user holds across all pictures
}
```

**Response `200` — picture tags (`picture_id` set, `with_sources=false`):**

```ts
{
    tags: string[];  // deepest distinct tag paths for this picture (folded — ancestors omitted)
}
```

**Response `200` — picture tags with provenance (`picture_id` set, `with_sources=true`):**

```ts
{
    tags: Array<{
        path: string;
        sources: Array<{
            source: TagSource;   // "manual" | "rule" | "segment" | "share_mapping" | "incoming_share"
            source_id: string | null;  // tagging service UUID, or null for manual
        }>;
    }>;
}
```

`TagSource` values:

- `"manual"` — assigned by the user directly
- `"rule"` — assigned by a `RuleTaggingService`
- `"segment"` — assigned by a `SegmentationTaggingService`
- `"share_mapping"` — assigned by a `SharedTagMappingService`
- `"incoming_share"` — assigned automatically when a share was accepted (the `/SharedToMe/...` tag)

---

#### `PATCH /api/authenticated/tags`

Add or remove tags on one or more pictures.

**Request:**

```ts
{
    picture_ids: string[];   // required
    add_tags ? : string[];     // ltree paths (dot-separated) to add as "manual" tags
    remove_tags ? : string[];  // ltree paths to remove — only removes "manual" tags
}
```

Tag paths must not start with `SharedToMe` (protected prefix).

**Response `200`:**

```ts
{
    ok: true
}
```

**Side-effects:** Pipeline is invalidated for all affected pictures and woken.

---

### 6.7 Tagging Services

The tagging pipeline is an ordered list of services that automatically assign tags to pictures based on rules. Services run in order; each has
optional `requires`/`excludes` gates. See `doc/01_GENERAL_SPECIFICATIONS.md §3` for full semantics.

#### `GET /api/authenticated/tagging-services`

List all tagging services with their rules, in pipeline execution order.

**Response `200`:** `ServiceDetailResponse[]` — tagged union on `service_type`.

```ts
type ServiceDetailResponse =
    | SharedTagMappingServiceDetail
    | RuleServiceDetail
    | SegmentationServiceDetail;

// Common fields on all service types:
interface ServiceBase {
    id: string;
    service_type: ServiceType;
    requires: string[];  // ltree paths — service only fires if picture has ALL of these tags
    excludes: string[];  // ltree paths — service only fires if picture has NONE of these tags
    enabled: boolean;
    position: number;    // execution order (lower = earlier)
    created_at: string;
    updated_at: string;
}

interface SharedTagMappingServiceDetail extends ServiceBase {
    service_type: "shared_tag_mapping";
    mappings: SharedTagMappingRule[];
}

interface RuleServiceDetail extends ServiceBase {
    service_type: "rule";
    rules: RuleTaggingRule[];
}

interface SegmentationServiceDetail extends ServiceBase {
    service_type: "segmentation";
    segments: SegmentationSegment[];
}

interface SharedTagMappingRule {
    id: string;
    incoming_share_id: string;
    assign_tag: string;     // ltree path — tag assigned to pictures from this share
    is_broken: boolean;     // true if the referenced IncomingShare was revoked
}

interface RuleTaggingRule {
    id: string;
    predicate: string;   // e.g. "gps_within_bbox(45.8, 6.8, 46.1, 7.1)"
    assign_tag: string;
}

interface SegmentationSegment {
    id: string;
    name: string;
    date_start: string;  // ISO 8601 datetime
    date_end: string;
    assign_tag: string;
    parent_segment_id: string | null;  // null = top-level segment
}
```

**Supported predicates for `RuleTaggingRule.predicate`:**

- `gps_within_bbox(lat_min, lat_max, lon_min, lon_max)` — GPS bounding box
- `capture_year(YYYY)` — year of capture date
- `capture_month(M)` — month of capture date (1–12)
- `filename_contains("string")` — case-sensitive substring match on filename

---

#### `GET /api/authenticated/tagging-services/{id}`

Get a single tagging service with rules.

**Path params:** `id: string`

**Response `200`:** `ServiceDetailResponse` (single item, same tagged-union shape).

**Errors:** 404 if not found or not owned by current user.

---

#### `POST /api/authenticated/tagging-services`

Create a new tagging service.

**Request:**

```ts
{
    service_type: ServiceType;   // "shared_tag_mapping" | "rule" | "segmentation"
    requires ? : string[];
    excludes ? : string[];
}
```

**Response `200`:**

```ts
interface ServiceResponse {
    id: string;
    service_type: ServiceType;
    requires: string[];
    excludes: string[];
    enabled: boolean;
    position: number;
    created_at: string;
    updated_at: string;
}
```

New service starts enabled at `position = max(existing)+1`.

**Side-effects:** Pipeline is woken — all pictures are dirty against the new service.

---

#### `PATCH /api/authenticated/tagging-services/{id}`

Update a service.

**Path params:** `id: string`

**Request:**

```ts
{
    enabled ? : boolean;
    requires ? : string[];  // replaces the entire current list
    excludes ? : string[];  // replaces the entire current list
}
```

**Response `200`:** `ServiceResponse` (flat, without rules).

**Side-effects:**

- Setting `enabled = false` immediately removes all tags this service assigned.
- Any change invalidates the service and wakes the pipeline for a full re-evaluation.

---

#### `DELETE /api/authenticated/tagging-services/{id}`

Delete a tagging service (cascades to all its rules).

**Path params:** `id: string`

**Query params:**
| Name | Type | Required | Description |
|---|---|---|---|
| `promote_tags` | `boolean` | **yes** | When `true`, the service's assigned tags are converted to manual user tags (user keeps the curation). When
`false`, they are deleted. |

**Response `200`:**

```ts
{
    deleted: true
}
```

---

#### `POST /api/authenticated/tagging-services/reorder`

Set the execution order of Rule and Segmentation services. `SharedTagMapping` services are always first and must not be included.

**Request:**

```ts
{
    ordered_ids: string[];  // complete list of Rule + Segmentation service UUIDs in desired order
}
```

**Response `200`:**

```ts
{
    reordered: true
}
```

---

#### `POST /api/authenticated/tagging-services/{id}/mappings`

Add a mapping rule to a `shared_tag_mapping` service.

**Path params:** `id: string`

**Request:**

```ts
{
    incoming_share_id: string;  // UUID of the IncomingShare to map
    assign_tag: string;          // ltree path to assign (no protected prefixes)
}
```

**Response `200`:** `SharedTagMappingRule`

**Errors:** 400 if service is not `shared_tag_mapping` type; 404 if not found.

---

#### `DELETE /api/authenticated/tagging-services/{id}/mappings/{rule_id}`

Remove a mapping rule.

**Path params:** `id: string`, `rule_id: string`

**Response `200`:**

```ts
{
    deleted: true
}
```

---

#### `POST /api/authenticated/tagging-services/{id}/rules`

Add a rule to a `rule` tagging service.

**Path params:** `id: string`

**Request:**

```ts
{
    predicate: string;   // validated predicate expression
    assign_tag: string;  // ltree path (no protected prefixes)
}
```

**Response `200`:** `RuleTaggingRule`

**Errors:** 400 if service is not `rule` type, or predicate syntax is invalid; 404 if not found.

---

#### `DELETE /api/authenticated/tagging-services/{id}/rules/{rule_id}`

Remove a rule from a `rule` service.

**Path params:** `id: string`, `rule_id: string`

**Response `200`:**

```ts
{
    deleted: true
}
```

---

#### `POST /api/authenticated/tagging-services/{id}/segments`

Add a date-range segment to a `segmentation` service.

**Path params:** `id: string`

**Request:**

```ts
{
    name: string;
    date_start: string;              // ISO 8601 datetime
    date_end: string;                // must be after date_start
    assign_tag: string;              // ltree path (no protected prefixes)
    parent_segment_id ? : string;      // UUID of parent segment (for nesting)
}
```

**Response `200`:** `SegmentationSegment`

**Errors:** 400 if service is not `segmentation` type, or `date_end <= date_start`; 404 if not found.

---

#### `DELETE /api/authenticated/tagging-services/{id}/segments/{segment_id}`

Remove a segment (cascades to all child segments).

**Path params:** `id: string`, `segment_id: string`

**Response `200`:**

```ts
{
    deleted: true
}
```

---

### 6.8 Sharing

Shares allow one user to give another access to all pictures under a tag path. The sharing model is federated — the recipient may be on a different
instance.

See `doc/01_GENERAL_SPECIFICATIONS.md §6` for full sharing semantics including ShareBack, transitive sharing, and revocation.

#### `POST /api/authenticated/shares/outgoing`

Create an outgoing share.

**Request:**

```ts
{
    tag_path: string;                // ltree path — all pictures under this tag are shared
    recipient_username: string;
    recipient_instance: string;      // global domain (e.g. "other.example.com")
    allow_share_back ? : boolean;      // default true — if true, auto-accepts a ShareBack from the recipient
    future ? : boolean;                // default true — automatically share pictures added to the tag later
    shareback_of ? : string;           // UUID of an IncomingShare — marks this as a ShareBack
}
```

**Response `200`:**

```ts
interface ShareResponse {
    id: string;
    tag_path: string;
    recipient_username: string;
    recipient_instance: string;
    status: ShareStatus;
    allow_share_back: boolean;
    future: boolean;
}
```

`ShareStatus` values and meaning:

- `"pending"` — share announced, waiting for recipient to accept
- `"pending_first_announcement"` — accepted; pipeline is announcing current pictures
- `"active"` — fully operational; pictures are being announced
- `"errored"` — a delivery failed; pipeline will retry automatically with backoff
- `"revoked"` — sender revoked the share
- `"tombstoned"` — recipient rejected the share

**Side-effects:** The federation handshake and share announcement run synchronously. If federation delivery fails, the share creation is rolled back.

---

#### `GET /api/authenticated/shares/outgoing`

List all outgoing shares.

**Response `200`:** `ShareResponse[]`

---

#### `POST /api/authenticated/shares/outgoing/{id}/revoke`

Revoke an outgoing share. Immediately removes shared pictures at the recipient and invalidates presign tokens.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    revoked: true
}
```

---

#### `GET /api/authenticated/shares/incoming`

List all incoming shares.

**Response `200`:**

```ts
interface IncomingShareResponse {
    id: string;
    sender_username: string;
    sender_instance: string;
    outgoing_share_id: string;
    status: ShareStatus;
    allow_share_back: boolean;       // whether the sender allows a ShareBack
    local_mapping_service_id: string | null;  // linked SharedTagMappingService (if set up)
}
```

---

#### `POST /api/authenticated/shares/incoming/{id}/accept`

Accept an incoming share (`pending → active`). Pictures are announced asynchronously by the sender's pipeline after acceptance.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    accepted: true
}
```

---

#### `POST /api/authenticated/shares/incoming/{id}/reject`

Reject an incoming share. Moves it to `tombstoned` status.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    rejected: true
}
```

---

### 6.9 Hierarchies

A hierarchy maps a filtered view of the tag graph to a navigable directory tree. It stores **no
pictures** — every directory resolves to a tag-set predicate and its picture list is derived live.
The `config` is an ordered tree of nodes (`mirror` / `query` / `static`); see
`doc/01_GENERAL_SPECIFICATIONS.md §4` and `doc/features/05_hierarchies.md` for the full model.

Write operations (move/copy/upload/delete) ship with WebDAV and are **not** part of this API yet;
the `config` already declares the write-back model so no schema change is needed when WebDAV lands.

#### `config` shape

```ts
interface HierarchyConfig {
    version: number;                              // schema version (currently 1)
    safeDeleteMode: "singleBranch" | "fullDelete"; // hierarchy default
    naming: "original" | "date" | "id";           // hierarchy default (WebDAV file naming)
    writeBack: boolean;                            // master switch; false ⇒ entire hierarchy read-only
    nodes: Node[];                                 // ordered root-level tree
}

// Common node fields + a kind discriminator.
type Node =
    | { id: string; name?: string; naming?: NamingStrategy; safeDeleteMode?: SafeDeleteMode;
        kind: "mirror"; tagRoot: string; keepDir?: boolean;
        collapsed?: string[]; exclude?: string[]; }
    | { id: string; name: string; naming?: NamingStrategy; safeDeleteMode?: SafeDeleteMode;
        kind: "query"; match?: "all" | "any"; include?: string[]; exclude?: string[];
        matchUntagged?: boolean; writeBack?: WriteBack | null; children?: Node[]; }
    | { id: string; name: string; naming?: NamingStrategy; safeDeleteMode?: SafeDeleteMode;
        kind: "static"; children?: Node[]; };

interface WriteBack {
    onAdd: Array<{ op: "assign" | "remove"; path: string }>;
    onRemove: Array<{ op: "assign" | "remove"; path: string }>;
}
```

- **`mirror`** — expands the live tag subtree under `tagRoot`. `keepDir` keeps the `tagRoot` label as
  a directory level; `collapsed` subtrees roll their pictures up to the nearest enabled ancestor;
  `exclude` subtrees are removed entirely. `mirror` is a leaf in the authored JSON (children are
  tag-derived). Tag paths in `collapsed`/`exclude` must be under `tagRoot`.
- **`query`** — explicit predicate; may nest. Effective predicate = own ∧ all ancestors. `match`
  combines `include` (AND/OR); `exclude` rejects; `matchUntagged: true` means "no stored tag of any
  source" (requires empty `include`/`exclude`). `writeBack: null` ⇒ read-only directory.
- **`static`** — pure container, no predicate, no direct pictures, read-only.

Validation (server-side, on create/update) rejects: duplicate node ids, duplicate sibling names,
`collapsed`/`exclude` not under `tagRoot`, `matchUntagged` with a non-empty `include`/`exclude`, and a
`writeBack` op-list that cannot satisfy/break the predicate.

#### `GET /api/authenticated/hierarchies`

List the user's hierarchies.

**Response `200`:** `Array<{ id: string; name: string; enabled: boolean }>`

#### `POST /api/authenticated/hierarchies`

Create a hierarchy. `config` defaults to an empty node tree when omitted; the server stores the
**normalized** config (defaults filled in).

**Request:**

```ts
{
    name: string;
    config ? : HierarchyConfig;
}
```

**Response `200`:**

```ts
interface HierarchyDetail {
    id: string;
    name: string;
    enabled: boolean;
    config: HierarchyConfig;
    created_at: string;
    updated_at: string;
}
```

**Errors:** 400 on invalid `config` or empty `name`; 409 if a hierarchy with that name already exists.

#### `GET /api/authenticated/hierarchies/{id}`

Get one hierarchy with its full `config`. **Response `200`:** `HierarchyDetail`. 404 if not found.

#### `PATCH /api/authenticated/hierarchies/{id}`

Update name / enabled / config (any subset; omitted fields unchanged). A supplied `config` is
re-validated.

**Request:** `{ name?: string; enabled?: boolean; config?: HierarchyConfig }`

**Response `200`:** `HierarchyDetail`.

#### `DELETE /api/authenticated/hierarchies/{id}`

**Response `200`:** `{ deleted: true }`. 404 if not found.

#### `GET /api/authenticated/hierarchies/{id}/tree`

Resolve the directory tree at a path (no pictures — cheap; for the sidebar).

**Query params:**
| Name | Type | Default | Description |
|---|---|---|---|
| `path` | `string` | `""` (root) | Slash-separated directory **names** (not ids), e.g. `Photos/Travel` |
| `depth` | `number` | `1` | How many levels of children to return |
| `counts` | `boolean` | `false` | When true, compute `picture_count` per directory and hide empty directories |

**Response `200`:**

```ts
{
    path: string;                  // echo of the resolved path
    directories: DirEntry[];
}

interface DirEntry {
    name: string;                  // append to `path` to navigate
    writable: boolean;
    child_count: number;           // number of child directories
    picture_count: number | null;  // null unless counts=true (count of this dir's direct files)
    children?: DirEntry[];         // present when depth > 1
}
```

Directory addressing uses **names**, not node ids. 404 if `path` does not resolve.

#### `GET /api/authenticated/hierarchies/{id}/browse`

Paginated pictures of one directory. The server resolves `path` into the directory's "most-specific
node wins" predicate and reuses the picture list machinery — the client only ever sends a `path`.

**Query params:** `path` (default root) plus the same pagination/filter params as
`GET /pictures`: `page`, `page_size`, `sort`, `order`, `include_deleted`, `owned_only`,
`shared_with_me`, `captured_after`, `captured_before`, `thumbnail`.

**Response `200`:** identical shape to `GET /pictures` (`{ total, page, page_size, items }`). A
`static` directory (no direct files) returns an empty page.

#### WebDAV mount (token management)

Each hierarchy can be mounted as a WebDAV drive at `{scheme}://{back_domain}/webdav/{slug}`
(`slug` = the slugified hierarchy name). The mount is authenticated with HTTP **Basic** — the
username is the `@user` and the password is the per-hierarchy **token** below. The token is
stored encrypted at rest and shown here so the owner can paste it into a client. See
`doc/features/06_webdav.md`.

```ts
interface WebdavResponse {
  url: string;          // mount URL — {scheme}://{back_domain}/webdav/{slug}
  token: string;        // Basic-auth password
  use_redirect: boolean; // true ⇒ reads 302-redirect to presigned URLs; false ⇒ backend proxies bytes
  enabled: boolean;      // mount is disabled when the hierarchy is disabled
}
```

#### `GET /api/authenticated/hierarchies/{id}/webdav`

Returns the mount URL and token, **minting a token on first access**.

**Response `200`:** `WebdavResponse`. 404 if the hierarchy is not found.

#### `POST /api/authenticated/hierarchies/{id}/webdav/regenerate`

Rotate the token (invalidates any currently-mounted client).

**Response `200`:** `WebdavResponse` with the new `token`.

#### `PATCH /api/authenticated/hierarchies/{id}/webdav`

Toggle the read strategy. **Request:** `{ use_redirect: boolean }`. **Response `200`:** `WebdavResponse`.

---

## 7. Admin Endpoints

All endpoints require a user JWT with `is_admin = true`. The admin check is on the `is_admin` JWT claim — there is no separate admin token type.

---

### `GET /api/admin/instance`

Instance health check.

**Response `200`:**

```ts
{
    global_domain: string;
    back_domain: string;
    db_connected: boolean;
    redis_connected: boolean;
    last_worker_activity_at: string | null;  // RFC3339 timestamp
}
```

---

### `GET /api/admin/stats`

Instance-wide analytics. **Cached for 60 seconds** in Redis.

**Response `200`:**

```ts
{
    user_count: number;
    owned_picture_count: number;
    received_picture_count: number;
    total_storage_bytes: number;
    job_counts: {
        pending: number;
        processing: number;
        completed: number;
        failed: number;
    }
    ;
    errored_share_count: number;
    pending_first_announcement_count: number;
    dirty_picture_count: number;
    last_worker_activity_at: string | null;
}
```

---

### `GET /api/admin/consistency`

Consistency check — identifies stuck/broken system state.

**Response `200`:**

```ts
{
    stuck_exif_pending_count: number;     // pictures with exif_sync_status='pending' but no active edit job
    pictures_without_thumbnail_count: number;  // pictures >30min old with no thumbnails
    broken_mapping_count: number;         // SharedTagMappingService rows whose IncomingShare was revoked
}
```

---

### `GET /api/admin/users`

List all users with storage usage.

**Response `200`:**

```ts
interface AdminUserResponse {
    id: string;
    username: string;
    email: string;
    display_name: string;
    is_admin: boolean;
    storage_bytes: number;
}
```

Returns `AdminUserResponse[]`.

---

### `POST /api/admin/users`

Create a user (admin override, bypasses resolver routing).

**Request:**

```ts
{
    username: string;
    email: string;
    display_name: string;
    password: string;
    is_admin ? : boolean;  // default false
}
```

**Response `200`:** `AdminUserResponse`

---

### `PATCH /api/admin/users/{id}`

Update a user's display name or admin role.

**Path params:** `id: string`

**Request:**

```ts
{
    display_name ? : string;
    is_admin ? : boolean;
}
```

**Response `200`:** `AdminUserResponse`

---

### `DELETE /api/admin/users/{id}`

Delete a user.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    deleted: true
}
```

---

### `GET /api/admin/users/{id}/stats`

Per-user analytics. **Cached for 120 seconds** in Redis.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    owned_picture_count: number;
    received_picture_count: number;
    storage_bytes: number;
    job_counts: {
        pending: number;
        processing: number;
        completed: number;
        failed: number;
    }
    ;
    outgoing_share_counts: Record<ShareStatus, number>;
    incoming_share_counts: Record<ShareStatus, number>;
    dirty_picture_count: number;
    errored_share_count: number;
}
```

**Errors:** 404 if user not found.

---

### `GET /api/admin/users/{id}/shares`

Get all shares (outgoing and incoming) for a user. Useful for diagnosing stuck/errored shares.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    outgoing: OutgoingShareRow[];
    incoming: IncomingShareRow[];
}

interface OutgoingShareRow {
    id: string;
    owner_id: string;
    tag_path: string;
    recipient_username: string;
    recipient_instance: string;
    allow_share_back: boolean;
    future: boolean;
    status: ShareStatus;
    created_at: string;
    revoked_at: string | null;
}

interface IncomingShareRow {
    id: string;
    recipient_id: string;
    sender_username: string;
    sender_instance: string;
    outgoing_share_id: string;
    local_mapping_service_id: string | null;
    status: ShareStatus;
    allow_share_back: boolean;
    created_at: string;
    revoked_at: string | null;
}
```

**Errors:** 404 if user not found.

---

### `POST /api/admin/users/{id}/pipeline/wake`

Force-wake the tagging pipeline for a specific user immediately (bypasses the coalescing delay).

**Path params:** `id: string`

**Response `200`:**

```ts
{
    woken: true
}
```

**Errors:** 404 if user not found.

---

### `GET /api/admin/jobs`

List jobs with optional filters.

**Query params:**
| Name | Type | Default | Description |
|---|---|---|---|
| `status` | `JobStatus` | — | Filter by job status |
| `type` | `JobType` | — | Filter by job type |
| `user_id` | `string` | — | Filter by owner UUID |
| `limit` | `number` | `50` | Max results (1–200) |
| `offset` | `number` | `0` | Pagination offset |

**Response `200`:** `AdminJobResponse[]`

```ts
interface AdminJobResponse {
    id: string;
    owner_id: string;
    owner_username: string;
    job_type: JobType;
    status: JobStatus;
    retry_count: number;
    max_retries: number;
    error_message: string | null;
    picture_id: string | null;
    claimed_by: string | null;
    created_at: string;
    started_at: string | null;
    completed_at: string | null;
}
```

Note: `config` and `result` JSONB columns are not included in admin responses.

---

### `GET /api/admin/jobs/stale`

List jobs currently stuck in `processing` state beyond the processing timeout (default 600s).

**Response `200`:** `AdminJobResponse[]` ordered by `started_at ASC`.

---

### `POST /api/admin/jobs/{id}/reset`

Force-reset a non-completed job back to `pending`. Clears claim state and resets `retry_count` to 0.

**Path params:** `id: string`

**Response `200`:** `AdminJobResponse` (updated row).

**Errors:** 404 if not found or already `completed`.

---

### `POST /api/admin/jobs/{id}/cancel`

Permanently fail a job (admin force-cancel). Sets `status = "failed"`.

**Path params:** `id: string`

**Response `200`:** `AdminJobResponse` (updated row).

**Errors:** 404 if not found or already in a terminal state.

---

### `GET /api/admin/shares/errored`

List all outgoing shares in `errored` state across all users.

**Response `200`:**

```ts
interface ErroredShareResponse {
    id: string;
    owner_id: string;
    owner_username: string;
    tag_path: string;
    recipient_username: string;
    recipient_instance: string;
    next_retry_at: string | null;
    last_error_at: string | null;
    created_at: string;
}
```

Returns `ErroredShareResponse[]`.

---

### `POST /api/admin/shares/outgoing/{id}/force-reconcile`

Clear the retry backoff on an `errored` or `pending_first_announcement` share and immediately wake the owner's pipeline.

**Path params:** `id: string`

**Response `200`:**

```ts
{
    reconcile_triggered: true
}
```

**Errors:** 404 if share not found or not in a recoverable state.

---

### `GET /api/admin/federation/instances`

List all remote federated instances known to this backend (derived from share records).

**Response `200`:**

```ts
interface FederationInstanceResponse {
    instance: string;               // global domain
    outgoing_share_count: number;
    incoming_share_count: number;
    errored_share_count: number;
}
```

Returns `FederationInstanceResponse[]`.

---

## 8. Federation & Worker Endpoints (for reference only)

These endpoints are called by other backend instances and workers respectively. The frontend **never calls these directly**. They are documented here
for completeness.

### Federation (`/api/federation/*`)

All require a federation JWT (pairwise, issued by the target instance).

| Method | Path                                  | Description                                                   |
|--------|---------------------------------------|---------------------------------------------------------------|
| `POST` | `/api/federation/auth/request`        | Request a federation JWT from another instance                |
| `POST` | `/api/federation/auth/grant`          | Receive a federation JWT                                      |
| `POST` | `/api/federation/shares/announce`     | Announce a new share                                          |
| `POST` | `/api/federation/shares/accept`       | Notify sender of share acceptance                             |
| `POST` | `/api/federation/shares/reject`       | Notify sender of share rejection                              |
| `POST` | `/api/federation/shares/revoke`       | Revoke a share (sender → recipient)                           |
| `POST` | `/api/federation/pictures/announce`   | Deliver picture announcements for an active share             |
| `POST` | `/api/federation/pictures/unannounce` | Remove specific pictures from a share                         |
| `POST` | `/api/federation/pictures/presign`    | Get presigned URLs using per-picture tokens (no JWT required) |

The `presign` endpoint is notable: it is called by the **recipient backend** on behalf of the recipient's frontend when fetching a picture owned by
the sender. The frontend does not call it directly — the `GET /api/authenticated/pictures/{id}/url` endpoint handles cross-instance presigning
transparently.

### Worker (`/api/worker/*`)

All require a worker JWT (`WORKER_JWT_SECRET`, 300s TTL).

| Method | Path                             | Description                                                              |
|--------|----------------------------------|--------------------------------------------------------------------------|
| `GET`  | `/api/worker/jobs/next`          | Claim next pending job; returns job + presigned S3 URLs + `claim_token`  |
| `POST` | `/api/worker/jobs/{id}/complete` | Report success; backend applies picture updates atomically               |
| `POST` | `/api/worker/jobs/{id}/fail`     | Report failure; auto-retries up to `max_retries` unless `permanent=true` |

---

## 9. WebFinger (`/.well-known/webfinger`)

Used for user identity resolution. The frontend calls this when it needs to find which backend hosts a `@username:domain` identity.

### `GET /.well-known/webfinger`

**Auth:** None. Response content type: `application/jrd+json`.

**Query params:**
| Name | Type | Required | Description |
|---|---|---|---|
| `resource` | `string` | yes | Must match `archypix:@<username>:<domain>` |

**Response `200`:**

```ts
{
    subject: string;   // "archypix:@username:domain"
    links: Array<{
        rel: "backend_url";
        href: string;    // the resolved backend URL (scheme + host)
    }>;
}
```

**Errors:**

- 400 if `resource` does not match the expected format.
- 404 if the domain does not match this instance's global domain.

The frontend should call this to resolve cross-instance picture owners before fetching their pictures. The resolved `href` is then used as the base
URL for federation API calls.

---

## 10. Shared Type Reference

```ts
// Job types
type JobType = "gen_thumbnail" | "ml_style" | "ml_people" | "ml_group_location" | "edit_picture";

// Job statuses
type JobStatus = "pending" | "processing" | "completed" | "failed";

// Share statuses
type ShareStatus =
    | "pending"                      // announced, awaiting recipient acceptance
    | "pending_first_announcement"   // accepted, pipeline is delivering pictures
    | "active"                       // fully operational
    | "errored"                      // delivery failed, pipeline will retry with backoff
    | "revoked"                      // sender revoked the share
    | "tombstoned";                  // recipient rejected the share

// Tagging service types
type ServiceType = "shared_tag_mapping" | "rule" | "segmentation";

// Versioning modes
type VersioningMode =
    | "none"             // never snapshot
    | "original_copy"    // snapshot the original once (on first edit)
    | "full_versioning"; // snapshot before every visual edit

// EXIF sync status
type ExifSyncStatus =
    | "synced"        // DB and file are in sync
    | "pending"       // edit_picture job is in flight reconciling the file
    | "unsupported";  // format cannot embed EXIF; DB is updated, file is not

// Picture variants (thumbnail sizes)
type PictureVariant = "original" | "small" | "medium" | "large";

// Tag sources (for provenance display)
type TagSource = "manual" | "rule" | "segment" | "share_mapping" | "incoming_share";

// Editable EXIF fields
type ExifField =
    | "captured_at"
    | "gps_lat"
    | "gps_lng"
    | "gps_alt"
    | "orientation"
    | "camera_brand"
    | "camera_model"
    | "focal_length_mm"
    | "f_number"
    | "iso_speed"
    | "exposure_time_num"
    | "exposure_time_den";
```

---

## 11. Key Frontend Behaviours

### Tag path representation

- **Display form:** `/Photos/Travel/Alps` (slash-separated, slash prefix)
- **Wire form:** `Photos.Travel.Alps` (dot-separated, no prefix)
- All API requests and responses use wire form. Convert using `src/lib/utils.ts:TagPath`.

### Picture orientation

Thumbnails and originals are stored in **raw pixel orientation** — the worker does not bake the EXIF orientation into the generated files, and an
orientation edit does not regenerate thumbnails. The list (`orientation`) and detail (`orientation`) responses carry the EXIF orientation value (1–8);
the client rotates the image at display time to show it correctly (90°/270° orientations also transpose the displayed width/height). Rotating a
picture
is a normal EXIF edit (`set: { orientation }`) — the new value reconciles into the original file via the `edit_picture` job, while the displayed
rotation updates immediately from the DB value.

### Presigned URL caching

Presigned URLs are valid for ~15 minutes. Cache them in TanStack Query with `staleTime` set to at most 10 minutes. Do not fetch a new URL on every
render.

### Optimistic picture listing with `thumbnail`

Use the `thumbnail` query param on `GET /api/authenticated/pictures` to embed presigned thumbnail URLs in list items. This saves one round-trip per
picture compared to calling `GET /api/authenticated/pictures/{id}/url` for each item.

### Batch upload pattern

For multi-file uploads, the recommended flow is:

1. Call `POST /uploads/batch` with all filenames → receive all `(picture_id, presigned_url)` pairs.
2. Upload all files to S3 in parallel (recommend max 4 concurrent PUTs to avoid saturating the connection).
3. As each S3 PUT completes, immediately call `POST /uploads/{id}/complete` — do not wait for all files.
4. Pass `initial_tags` in the complete body to assign tags to every uploaded picture without a separate `PATCH /tags` round-trip.

### Pipeline wakeup side-effects

Several mutations wake the tagging pipeline asynchronously:

- `POST /uploads/{id}/complete` — new picture (pipeline also re-evaluates `initial_tags` against services)
- `PATCH /tags` — manual tag change
- `PATCH /tagging-services/{id}` — service config change
- `POST /tagging-services` — new service created
- `DELETE /tagging-services/{id}` — service deleted

After these mutations, dirty pictures are re-evaluated in the background. The frontend does not need to poll — tags on pictures update eventually, and
the gallery can be refetched after a short delay or on next navigation.

### EXIF sync polling

After `POST /pictures/{id}/edit`, if `exif_sync_status = "pending"`, poll `GET /jobs/{job_id}` until `status = "completed"` or `"failed"` (the
`job_id` is in the edit response). Use exponential backoff (e.g. 1s, 2s, 4s, stop at ~30s).

### Received pictures (shares)

- `owned = false` in list/detail responses indicates a received picture.
- `owner_username` and `owner_instance` identify the true owner.
- The `GET /pictures/{id}/url` endpoint transparently handles cross-instance presigning (the response URL may point to the owner's backend).
- Received pictures can have manual tags and appear in segmentation/rule results; they cannot have EXIF edited.

### Share workflow

1. **Outgoing:** `POST /shares/outgoing` → share is `pending` → recipient accepts → share moves to `pending_first_announcement` → pipeline delivers
   pictures → share becomes `active`.
2. **Incoming:** share arrives as `pending` → user calls `POST /shares/incoming/{id}/accept` → pictures appear over time as the sender's pipeline
   delivers them.
3. **ShareBack:** when creating a share, pass `shareback_of = <incoming_share_id>`. If the original share had `allow_share_back = true`, the new share
   auto-activates and a `SharedTagMappingService` rule is automatically created for it on the sender's side.
