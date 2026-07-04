# API Reference

Primary source of truth for frontend development. See `03_BACKEND_ARCHITECTURE.md §F` for JWT and federation auth conventions.

---

## 1. Route Groups

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

Login returns `{ access_token, refresh_token }`. Attach to requests as `Authorization: Bearer <access_token>`. On 401, call `POST /api/auth/refresh`
once; if that also fails, redirect to `/login`. Both tokens are stored in `localStorage`.

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

---

## 3. Wire Format Conventions

**Tag paths** — dot-separated ltree form on the wire (`Photos.Travel.Alps`; display form is `/Photos/Travel/Alps`). Label chars: `[A-Za-z0-9_]`. `@`→
`_AT_`, `.`→`_DOT_` within a label. The `TagPath` helper in `src/lib/utils.ts` converts between forms. All tag fields in requests/responses use wire
form.

**Protected prefix** — `SharedToMe` is reserved; the API rejects tags starting with it. Use `allow_protected = false` in `TagPath` for manual user
input.

**Datetimes** — ISO 8601 / RFC3339 in UTC. EXIF `captured_at` may arrive as `YYYY:MM:DD HH:MM:SS` — the backend normalizes it.

**IDs** — UUID v4 strings.

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

**Errors:** `401` on invalid credentials (timing-equalized — the response does not reveal whether
the username exists). `429` when too many attempts are made for the same username within the
rate-limit window.

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

### `POST /api/public/register`

Register a new user. This is the **single registration path the frontend uses** regardless of topology: a standalone backend (`USE_RESOLVER=false`)
serves it directly, and the resolver exposes the same path on its registration handler (picks a backend and forwards). On a backend running behind a
resolver (`USE_RESOLVER=true`) this route returns 400 — but the frontend targets the *global domain*, which is the resolver in that topology.
Passwords
must be at least 8 characters and the email must be syntactically valid (`400` otherwise). Rate-limited per source IP (`429` past the window).

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
  trash_retention_days: number;   // days a trashed owned picture is kept before physical purge (default 30)
    created_at: string;
    updated_at: string;
}
```

---

#### `PATCH /api/authenticated/settings`

Update settings. Both fields are optional; an omitted field keeps its current value.

**Request:**

```ts
{
  versioning_mode ? : VersioningMode;
  trash_retention_days ? : number;  // 1–3650; 400 if out of range
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

Presigns multiple upload slots in one round-trip. Returns results in the same order as the input array. Capped at 100 files per call.

**Request:**

```ts
{
  files: Array<{
    filename: string;       // non-empty
    file_hash?: string;     // SHA-256 lowercase hex of the bytes — enables upload-time dedup
  }>;                         // 1–100 entries
  initial_tags ? : string[];    // ltree wire-form paths — assigned (manual) to deduplicated pictures
  upload_label ? : string;      // single ltree label (`Uploaded.YYYY_MM_DD_HH_MM`), fixed per batch
                              // by the front — tags the import (feature 15, see below)
}
```

**Response `200`:**

```ts
Array<{
  picture_id: string;          // new picture (for a fresh file) or the existing one (for a dedup)
  presigned_url: string | null; // PUT the bytes here; null when duplicate is true
  duplicate: boolean;          // true ⇒ the hash already matched an existing owned picture
  was_deleted: boolean;        // true ⇒ the matched picture is in the trash (NOT auto-restored)
}>
```

**Deduplication.** When a file carries a `file_hash` that already matches one of the caller's
**owned** pictures, that slot comes back with `duplicate: true`, a `null` `presigned_url`, and
`picture_id` set to the existing picture — the client must **not** upload it. A matched picture that
was **trashed** comes back with `was_deleted: true` and is **not** un-deleted (feature 15); the client
surfaces these and offers to restore them. Any `initial_tags` are assigned (as `manual` tags) to those
existing pictures atomically, so re-uploading a photo still lands the user's intended tags on the
copy they already hold. The pipeline is woken when a tag assignment happened. Dedup also
applies **within a single batch**: two files sharing a hash (even when neither is in the DB yet) mint
one slot — the later copies come back `duplicate: true` pointing at that first slot's `picture_id`.
New (non-duplicate) files get a normal slot and receive their `initial_tags` later, on `complete`.

**Import tagging (feature 15).** When `upload_label` is set, duplicates are tagged here:
`<label>.AlreadyExisting` (live) or `<label>.AlreadyExisting.Deleted` (trashed). Brand-new files are
tagged with the bare `<label>` on `complete`. The label must be a single ltree label (no `.`); the
front fixes it once per batch so the whole import shares one date. The `<label>.AlreadyExisting.Deleted`
tag is assigned even though the picture stays trashed, so the user can find and restore that subset.

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
    file_size ? : number;      // bytes (i64) — advisory only; the authoritative size is read from S3
    file_hash ? : string;      // SHA-256 lowercase hex of the file — provisional ETag/dedupe key
    width ? : number;          // pixels (i32)
    height ? : number;
    exif_data ? : object;      // arbitrary EXIF key-value pairs
    captured_at ? : string;    // ISO 8601 datetime
    initial_tags ? : string[]; // ltree wire-form paths — assigned as manual tags atomically with picture creation
    upload_label ? : string;   // single ltree label (`Uploaded.YYYY_MM_DD_HH_MM`) — also assigned as a manual tag (feature 15)
  defer_pipeline ? : boolean; // default false — when true, this completion does NOT wake the pipeline
}
```

All fields are optional — the backend fills in EXIF fields from worker extraction if omitted.
`initial_tags` paths are validated and must not start with `SharedToMe` (reserved prefix); an
invalid path returns `400`. `file_size` is **advisory**: the backend reads the real size from S3
(`HEAD`) and stores that, so a client cannot under-report it. `file_hash` should be the SHA-256
(lowercase hex) of the uploaded bytes — the same digest the worker computes — and is stored as a
provisional ETag/dedupe key until `gen_thumbnail` re-confirms it.

The completion wakes the pipeline through the **debounced** path, so a batch upload's per-file
completions automatically coalesce into a single pipeline run (`PIPELINE_DEBOUNCE_MS` window) — no
need to defer and wake once at the end. `defer_pipeline: true` is an opt-out for a caller that wants
to drive the wake itself (e.g. via `POST /pictures/pipeline/wake`).

**Response `200`:**

```ts
{
    id: string
}  // picture UUID
```

**Side-effects:** creates the picture row, assigns any `initial_tags` as `manual` source tags, enqueues a `gen_thumbnail` job (EXIF extraction +
thumbnail generation), and wakes the pipeline (unless `defer_pipeline` is true). All of these happen atomically in a single DB transaction.

#### `POST /api/authenticated/pictures/pipeline/wake`

Explicitly wake the caller's tagging pipeline. Used by the tagging-editor "Force run" control and by
any caller that completed uploads with `defer_pipeline: true` (the upload path otherwise wakes the
pipeline via the debounced window on its own).

**Response `200`:** `{ woken: true }`

---

### 6.3 Pictures — List & Details

#### `GET /api/authenticated/pictures`

Paginated picture list.

**Query params:**
| Name | Type | Default | Description |
|---|---|---|---|
| `page` | `number` | `1` | Page number (1-indexed) |
| `page_size` | `number` | `50` | Items per page |
| `sort` | `"captured_at" \| "ingested_at" \| "updated_at" \| "file_size" \| "filename"` | `"ingested_at"` | Sort field. Ordering is stable (`NULLS LAST`, `id` tiebreaker) |
| `order` | `"asc" \| "desc"` | `"desc"` | Sort direction |
| `include_tags` | `string` | — | Comma-separated ltree paths the picture must match (inclusive `<@`), combined per `match`. For a single tag, pass one entry |
| `exclude_tags` | `string` | — | Comma-separated ltree paths; reject the picture if it has any (inclusive) |
| `exact` | `string` | — | Comma-separated ltree paths matched **exactly** (`tag_path = p`, no descendants) — strict tag navigation; combined with `include`/`exclude` per `match` |
| `match` | `"all" \| "any"` | `"all"` | Combinator over `include_tags`/`exact` (`all` = AND, `any` = OR) |
| `untagged` | `boolean` | `false` | Only pictures with no stored tag of any source. Mutually exclusive with `include_tags`/`exclude_tags`/`exact` |
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
  mime_type: string | null;      // lets the client flag playable media (video/audio) in the grid
    width: number | null;
    height: number | null;
    captured_at: string | null;
    ingested_at: string;
    blurhash: string | null;
    orientation: number | null;    // EXIF orientation (1–8); thumbnails are raw pixels — the client rotates them
    thumbnail_url: string | null;  // presigned URL for the `thumbnail` variant; null when that param
                                   // is unset OR the picture has no generated thumbnail (pending, or
                                   // a non-thumbnailable format like a PDF) — render a file-type icon
    owned: boolean;                // false for received (shared-to-me) pictures
    owner_username: string | null; // set when owned=false
    owner_instance: string | null; // global domain of the owning instance
    exif_sync_status: ExifSyncStatus;
  deleted_at: string | null;        // the holder's own local soft-delete (trash); null when not trashed
  owner_deleted_at: string | null;  // received only: the owner's soft-delete (grace-window badge)
  owner_purge_at: string | null;    // received only: the owner's announced purge deadline
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
  exif_data: object;             // arbitrary EXIF fields (camera make/model, focal length, etc.);
                                 // for video, read-only tech metadata: duration_s, video_codec,
                                 // audio_codec, frame_rate. Video EXIF edits are DB-only (unsupported sync)
    exif_sync_status: ExifSyncStatus;
    owner_username: string | null;
    owner_instance_domain: string | null;
  deleted_at: string | null;          // the holder's own local soft-delete (trash); null when not trashed
  owner_deleted_at: string | null;    // received only: the owner's soft-delete (grace-window badge)
  owner_purge_at: string | null;      // received only: the owner's announced purge deadline
  local_exif_overrides: object | null;// received only: the recipient's sticky per-field EXIF overrides
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

- `small` — WebP thumbnail, ~150px height
- `medium` — WebP thumbnail, ~500px height
- `large` — WebP thumbnail, ~1000px height
- `original` — original uploaded file at full resolution

**Response `200`:**

```ts
{
    url: string | null;   // null for a thumbnail variant (small/medium/large) when the picture has
                          // no generated thumbnail (pending, or non-thumbnailable). `original` is
                          // always a URL.
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

Batch EXIF edit over a **selection** (feature 14 §5–§6). See [§6.11](#611-batch-operations-feature-14)
for the `PictureSelection` model. Owned pictures take a **deferred-job** write-through (a single
set-based `UPDATE` stamps `exif_sync_status = "pending_job_creation"`; a background drain creates the
`edit_picture` reconcile jobs and flips them to `pending`). Received pictures take a recipient-local
override merge — or, in `mode: "suggest"` where the share grants editing, a propose-to-owner edit.
Because jobs are created by the drain, **no per-picture `job_id` is returned**; convergence is tracked
through the `exif_sync` histogram from `POST /pictures/aggregate`.

**Request:**

```ts
{
    selection?: PictureSelection;     // the selection (or use picture_ids)
    picture_ids?: string[];           // legacy explicit set (used when selection is absent)
    set ? : Partial<ExifOverrides>;
    empty ? : ExifField[];            // override to empty/null; owned + suggest fold it into clear
    clear ? : ExifField[];
    mode?: "local" | "suggest";       // default "local" (§6.1)
    dry_run?: boolean;                // default false
}
```

**Response `200` (apply):**

```ts
{
    affected: number;        // total pictures touched
    edited: number;          // owned, write-through (now pending_job_creation)
    suggested: number;       // received, proposed to owner (suggest mode + grant)
    local_override: number;  // received, recipient-local override
    unsupported: number;     // owned, format cannot embed EXIF
}
```

**Response `200` (dry-run):** the [§6.11 dry-run breakdown](#611-batch-operations-feature-14).

---

#### `POST /api/authenticated/pictures/{id}/exif/resync`

Re-enqueue a stuck EXIF sync (picture stuck in `exif_sync_status = "pending"` with no active job).

**Path params:** `id: string`

**Response `200`:** Full `Job` object (the newly enqueued job).

---

#### `POST /api/authenticated/pictures/{id}/exif`

Edit a **received** picture's EXIF (`set`/`clear`, same shape as an owned edit) in one of two modes
(`doc/features/10_recipient_exif_editing.md §4.1`):

```ts
{
    mode?: "local" | "propose";   // default "local"
    set?: Partial<ExifOverrides>;
    empty?: ExifField[];          // local mode: claim-as-empty (null)
    clear?: ExifField[];
}
```

- `mode: "local"` (default) — a **recipient-local** override. DB-only; no `edit_picture` job, no file
  reconcile (the recipient does not own the file). Three per-field verbs: `set` claims a sticky
  value; `empty` claims the field as **empty/`null`**, shadowing a present owner value with emptiness;
  `clear` drops the claim so the owner's value flows through again. The effective `exif_data` (+
  promoted columns) is `merge(owner snapshot, overrides)`. Always permitted. Returns `200`.
- `mode: "propose"` — **propose the edit to the owner**, who auto-applies it to the authoritative
  picture and re-announces so all recipients converge (owner is the serialization point;
  last-write-wins). Requires the incoming share to grant editing
  (`IncomingShareResponse.allow_exif_edit = true`), else `403`. On success the proposed fields'
  local overrides are cleared (the owner's value is authoritative). Any `empty` fields are folded
  into `clear` (owner-side clear nulls the column). The change lands asynchronously, so this returns
  `202 Accepted`.

**Path params:** `id: string` — must be a received (`owned = false`) picture.

**Response `200` (local) / `202` (propose):**

```ts
{
    id: string;
    captured_at: string | null;
    gps_lat: number | null;
    gps_lng: number | null;
    gps_alt: number | null;
    orientation: number | null;
    exif_data: object;                  // the materialised merge
    local_exif_overrides: object | null;
    updated_at: string;
}
```

**Errors:** `400` if the picture is owned (use `/edit` instead); `403` (`propose` only) if the share
does not authorise editing; `404` if not found; `409` (`propose` only) if the owner's picture is
still in initial extraction.

---

### 6.5 Pictures — Trash

Soft-delete is deferred for owned pictures (purged after `trash_retention_days`) and local-only for
received pictures (never physically deleted). Trashing an **owned** shared picture keeps it in share
coverage and re-announces it carrying the owner-deletion lifecycle (recipients show a grace-window
badge) until the purge sweep removes it. See `doc/features/09_trash_and_exif_overrides.md §5`.

#### `POST /api/authenticated/pictures/{id}/trash`

Soft-delete a picture the user holds (owned or received).

**Response `200`:** `{ id: string; deleted_at: string }`

#### `POST /api/authenticated/pictures/{id}/restore`

Restore a soft-deleted picture (clears `deleted_at`). For an owned picture this re-announces with the
lifecycle flag cleared.

**Response `200`:** `{ id: string; deleted_at: null }`

**Errors (both):** `404` if the user holds no such picture.

#### `POST /api/authenticated/pictures/trash` · `POST /api/authenticated/pictures/restore`

Batch soft-delete / restore over a **selection** (feature 14 §6). One set-based write (no per-picture
loop). See [§6.11](#611-batch-operations-feature-14) for the request/response shape. Restore must
target a selection that includes the trashed rows (e.g. the trash view's `include_deleted` query, or
explicit ids).

#### `POST /api/authenticated/pictures/{id}/copy`

Copy ("rescue") a received (or owned) picture into the caller's library as a **new, independent owned
picture** (feature 11 §3). The new picture never reuses the source id; its `copy_source_*` records
the provenance **root** (the genuine original's owner identity, resolved across copy chains). Bytes
are copied server-side — within S3 when the source file lives on this backend (owned or same-backend
received), or fetched via the source's per-picture presign for a cross-instance owner (which must be
reachable). EXIF is seeded from the source's *effective* values at copy time (a copy is a snapshot);
`gen_thumbnail` then fills `content_hash`/`file_hash`/thumbnails and the dedup reconciler runs.

**Path params:** `id: string` — a picture the caller holds.

**Response `200`:** `{ id: string }` — the new owned picture's id.

**Errors:** `404` if the caller holds no such picture; `401`/`5xx` if a cross-instance owner is
unreachable. Once owned, the copy behaves like any owned picture (shareable, editable, versioned).

#### `GET /api/authenticated/pictures/{id}/copies`

The picture's **content-dedup group** (feature 11 §5.5): the visible survivor plus its hidden
siblings (duplicates, trashed, rejected). Each row carries both hashes so the client can tell
"same image, EXIF-only difference" (same `content_hash`, different `file_hash`) from "different
image".

**Response `200`:**

```ts
{
  copies: Array<{
    id: string;
    filename: string | null;
    content_hash: string | null;
    file_hash: string | null;
    state: "live" | "manual" | "boomerang" | "content_dedupe" | "deleted";
    updated_at: string;            // last edit
    owned: boolean;
    owner_username: string | null; // received rows
    owner_instance: string | null;
    copy_source_owner_username: string | null;  // a physical copy's provenance root
    copy_source_owner_instance: string | null;
    copy_source_picture_id: string | null;
  }>;
}
```

`live` = the shown survivor; `manual` = the single recoverable trash representative; `content_dedupe`
/ `boomerang` = hidden (a dedup duplicate, or a rejected copy of content the user deleted).

#### `POST /api/authenticated/pictures/{id}/copies/keep`

Make this picture the **kept (live) survivor** of its content-dedup group, hiding the others as
`content_dedupe` (lifting any rejection). The reconciler leaves a correct single-live group
untouched, so the choice sticks. **Response `200`:** `{ kept: string }`. `404` if not held.

The picture-detail response (`GET /pictures/{id}`) additionally carries `content_hash`,
`copy_source_owner_username`, `copy_source_owner_instance`, and `copy_source_picture_id` (all `null`
for a normal upload/received row). **Content dedup** is automatic and transparent: byte-identical
copies are grouped by `content_hash` (or `file_hash` for unstrippable formats), one live survivor is
kept and the rest hidden, and a copy matching content the user previously **manually** deleted is
routed straight to (recoverable) trash. None of this needs a client call — hidden rows are simply
excluded from every default view. See `doc/features/11_physical_copy_and_dedup.md`.

---

### 6.6 Pictures — Jobs

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

### 6.7 Tags

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

Add or remove tags across a **selection** (feature 14 §6.4). See
[§6.11](#611-batch-operations-feature-14) for the `PictureSelection` model. Removal only affects
`manual` rows, so the removable count reflects `manual_count`, not `count`.

**Request:**

```ts
{
    selection?: PictureSelection;   // the selection (or use picture_ids)
    picture_ids?: string[];         // legacy explicit set (used when selection is absent)
    add_tags ? : string[];     // ltree paths (dot-separated) to add as "manual" tags
    remove_tags ? : string[];  // ltree paths to remove — only removes "manual" tags
    dry_run?: boolean;         // default false
}
```

Tag paths must not start with `SharedToMe` (protected prefix).

**Response `200` (apply):** `{ ok: true; affected: number }`

**Response `200` (dry-run):** the [§6.11 dry-run breakdown](#611-batch-operations-feature-14)
(`added` = pictures that gain a tag; `removed` = pictures holding a manual row under a removed path).

**Side-effects:** Pipeline is invalidated for all affected pictures and woken.

#### `POST /api/authenticated/tags/rename`

Rename a tag subtree everywhere the user references it (edge case §7, "Tag rename cascade"). A real
search-and-replace: manual picture tags, outgoing-share tags, tagging-service gates + config
(SharedTagMapping included), and hierarchy configs all have the `old_tag` prefix swapped for
`new_tag`. Changed services are invalidated and covered pictures marked dirty; the pipeline is woken
to re-derive service tags and re-announce shares under the renamed tag (the share tracking table is
untouched, so any pending announce/unannounce delta survives).

**Request:**

```ts
{
    old_tag: string;   // ltree path (dot-separated), non-reserved
    new_tag: string;   // ltree path (dot-separated), non-reserved
}
```

Both paths must be valid non-`SharedToMe` ltree paths, must differ, and neither may be an ancestor of
the other.

**Response `200`:** `{ ok: true }` — the cascade runs asynchronously (tag-rename routine); the ack is
immediate.

**Side-effects:** Runs the tag-rename cascade, then wakes the pipeline if any service/share changed or
any picture was marked dirty.

---

### 6.8 Tagging Services

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
    name: string;        // user-facing label (may be empty; UI falls back to a type label)
    service_type: ServiceType;
    requires: string[];  // ltree paths — service only fires if picture has ALL of these tags
    excludes: string[];  // ltree paths — service only fires if picture has NONE of these tags
    enabled: boolean;
    position: number;    // execution order (lower = earlier)
    created_at: string;
    updated_at: string;
}

// One service per incoming share (feature 20 §10.1). `is_broken` is **derived** (not stored):
// true when the referenced incoming share is absent or no longer active.
interface SharedTagMappingServiceDetail extends ServiceBase {
    service_type: "shared_tag_mapping";
    incoming_share_id: string;
    assign_tags: string[];   // ltree paths assigned to pictures from this share
    is_broken: boolean;
}

interface RuleServiceDetail extends ServiceBase {
    service_type: "rule";
    rules: RuleTaggingRule[];  // array order = display order
}

// Calendar segmentation (feature 20): a captured_at → tag partition operator.
interface SegmentationServiceDetail extends ServiceBase {
    service_type: "segmentation";
    config: SegmentationConfig;   // see doc/features/20_calendar_segmentation.md §3
}

interface RuleTaggingRule {
    id: string;
    predicate: RulePredicate;   // structured predicate tree (see below)
    assign_tag: string;
}
```

`SegmentationConfig` is the band-list config documented in
`doc/features/20_calendar_segmentation.md §3` (an ordered, flat list of bands, each rendering a
`captured_at`-derived tag under a single `root_tag`; first covering band wins).

The **`ServiceConfig`** accepted by create / `PUT …/config` is the type-specific payload, validated
against the service's `service_type`:

```ts
type ServiceConfig =
    | { rules: { id?: string; predicate: RulePredicate; assign_tag: string }[] }  // rule (id server-assigned if omitted)
    | SegmentationConfig                                                          // segmentation
    | { incoming_share_id: string; assign_tags: string[] };                       // shared_tag_mapping
```

**`RuleTaggingRule.predicate` — structured predicate tree (feature 13):**

A recursive JSON value combining logical nodes, spatial predicates, and typed field conditions.
See `doc/features/13_better_rules.md` for the full model; the backend validates the tree on
create/update (unknown keys, type-incompatible conditions, bad ranges, invalid regex, depth > 10).

```ts
type RulePredicate =
        | { and: RulePredicate[] }              // all children match (empty ⇒ always)
        | { or: RulePredicate[] }               // any child matches (empty ⇒ never)
        | { not: RulePredicate }                // inverts the child
        | { gps_bbox: { lat_min; lat_max; lon_min; lon_max } }
        | { gps_radius: { lat; lng; km } }
        | ({ field: string } & Record<string, unknown>);  // typed field condition leaf
```

Fields: `captured_at` / `ingested_at` / `updated_at` (date), `gps_lat`/`gps_lng`/`gps_alt`,
`iso_speed`, `f_number`, `focal_length_mm`, `exposure_time` (s), `orientation`, `camera_brand`,
`camera_model`, `filename`, `mime_type`, `file_size`, `width`, `height`, `is_owned` (bool).
Conditions by base type:

- **int/float** — `eq`, `min`, `max` (combine `min`+`max` for a range)
- **str** — `eq`, `contains`, `starts_with`, `ends_with`, `regex` (RE2). All string comparisons are
  **case-sensitive by default**; add a sibling `ignore_case: true` on the leaf to fold case (feature
  15 — replaces the old `eq_ic` operator and the previously-implicit case-insensitivity of
  `contains`/`starts_with`/`ends_with`)
- **date** — `year`, `month` (1–12), `season` (`spring|summer|autumn|winter`),
  `date_range: {from, to}` (each bound `YYYY-MM-DD` for a full day, or `YYYY-MM-DDTHH:MM:SS` for a
  precise instant), `time_range: {from, to}` (`HH:MM`, may cross midnight)
- **bool** — `eq`
- **any nullable field** — `is_present: boolean`

Example: `{"and": [{"field": "camera_brand", "eq": "fujifilm", "ignore_case": true}, {"field": "iso_speed", "min": 100, "max": 800}]}`.

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
    service_type: ServiceType;    // "rule" | "segmentation" | "shared_tag_mapping"
    name ? : string;              // optional user-facing label
    requires ? : string[];
    excludes ? : string[];
    config ? : ServiceConfig;     // type-specific config (see below); defaults to the empty config
}
```

`config` is the same type-specific object the service detail returns and `PUT …/config` accepts,
validated against `service_type`:

- **rule** → `{ rules: RuleInput[] }` where `RuleInput = { id?: string; predicate; assign_tag }`
  (omit `id` for new rules — the server assigns one). Defaults to `{ rules: [] }`.
- **segmentation** → a full `SegmentationConfig`. Defaults to an empty config rooted at `Photos`
  (replace it with `PUT …/config`).
- **shared_tag_mapping** → `{ incoming_share_id, assign_tags }`. `incoming_share_id` must be one of
  the caller's incoming shares (404 otherwise). No default — must be supplied.

Predicates, assigned tags, and segmentation bands are all validated; a `400` names the offending
node / band index.

**Response `200`:** `ServiceDetailResponse` (the created service with its normalized config).

New service starts enabled at `position = max(existing)+1`.

**Side-effects:** Pipeline is woken — all pictures are dirty against the new service.

---

#### `PATCH /api/authenticated/tagging-services/{id}`

Update a service.

**Path params:** `id: string`

**Request:**

```ts
{
    name ? : string;        // rename the service
    enabled ? : boolean;
    requires ? : string[];  // replaces the entire current list
    excludes ? : string[];  // replaces the entire current list
}
```

**Response `200`:** `ServiceResponse` (flat, without config).

**Side-effects:**

- Setting `enabled = false` immediately removes all tags this service assigned.
- Any change invalidates the service and wakes the pipeline for a full re-evaluation.

> The type-specific config (rules / segmentation bands / mapping tags) is **not** edited here — use
> `PUT /tagging-services/{id}/config` below.

---

#### `PUT /api/authenticated/tagging-services/{id}/config`

Replace a service's whole type-specific config — the **single, uniform** config-editing path for all
three service types. There are no per-rule / per-band / per-mapping sub-resources: the array (rules)
or band order in the submitted config *is* the stored order, so reordering / adding / removing is
just a `PUT` with the new array.

**Path params:** `id: string`

**Request:** `{ config: ServiceConfig }` — the full config for the service's stored type (same
shapes as the create `config`, validated identically).

**Response `200`:** `ServiceDetailResponse` (the service with its normalized config — rule `id`s
filled in).

**Errors:** `400` if the config is invalid (the message names the offending node / band index);
`404` if the service is not found, or (for a mapping) the `incoming_share_id` is not the caller's.

---

#### `DELETE /api/authenticated/tagging-services/{id}`

Delete a tagging service.

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

### 6.9 Sharing

Shares allow one user to give another access to all pictures under a tag path. The sharing model is federated — the recipient may be on a different
instance.

See `doc/01_GENERAL_SPECIFICATIONS.md §6` for full sharing semantics including ShareBack, transitive sharing, and revocation.

#### `POST /api/authenticated/shares/outgoing`

Create an outgoing share.

**Request:**

```ts
{
    tag_path: string;                // ltree path — all pictures under this tag are shared
  name: string;                    // required, ≤ 64 chars — short human-readable name shown to both parties
  message ? : string;                // optional, ≤ 1000 chars — free-text note for the recipient
    recipient_username: string;
    recipient_instance: string;      // global domain (e.g. "other.example.com")
    allow_share_back ? : boolean;      // default true — if true, auto-accepts a ShareBack from the recipient
    allow_exif_edit ? : boolean;       // default false — let recipients propose EXIF edits the owner auto-applies
    future ? : boolean;                // default true — automatically share pictures added to the tag later
  shareback_of ? : string;           // marks this as a ShareBack — the `outgoing_share_id` of the
                                     // incoming share being shared back (i.e. the original sender's
                                     // OutgoingShare id)
}
```

**Response `200`:**

```ts
interface ShareResponse {
    id: string;
    tag_path: string;
  name: string;
  message: string | null;
    recipient_username: string;
    recipient_instance: string;
    status: ShareStatus;
    allow_share_back: boolean;
    allow_exif_edit: boolean;        // whether recipients may propose EXIF edits the owner auto-applies
    future: boolean;
    shareback_of: string | null;     // provenance: the original OutgoingShare this share answers
    last_error_at: string | null;    // ISO — last failed announcement (while errored/recovering)
    next_retry_at: string | null;    // ISO — next scheduled retry (while errored/recovering)
    created_at: string;              // ISO
    revoked_at: string | null;       // ISO — when closed (revoked or rejected); null while live
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

**Errors:** `400` if `recipient_instance` is not a valid bare domain (schemes, ports, paths, IP
literals, and local domains are rejected — this guards the outbound federation call), or if `name` is
blank / exceeds 64 chars or `message` exceeds 1000 chars. `429` when the sender already holds the
maximum number of `pending` outgoing shares.

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
  name: string;                    // propagated from the sender's share
  message: string | null;          // propagated from the sender's share
    outgoing_share_id: string;
    status: ShareStatus;
    allow_share_back: boolean;       // whether the sender allows a ShareBack
  allow_exif_edit: boolean;        // propagated — whether the sender lets you propose EXIF edits
  future: boolean;                 // propagated — whether the sender auto-announces new pictures
  shared_tag_path: string | null;  // local /SharedToMe/<sender>/… tag (wire form) these land under;
                                   // set at creation, refreshed on each announcement
  last_announcement_received_at: string | null;  // ISO — last picture announcement from the sender
  shareback_of: string | null;     // provenance: the recipient's own OutgoingShare this answers
    local_mapping_service_id: string | null;  // linked SharedTagMappingService (if set up)
  created_at: string;              // ISO — when the incoming share was received
  revoked_at: string | null;       // ISO — when closed (revoked by sender or rejected); null while live
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

### 6.10 Hierarchies

A hierarchy maps a filtered view of the tag graph to a navigable directory tree. It stores **no
pictures** — every directory resolves to a tag-set predicate and its picture list is derived live.
The `config` is an ordered tree of nodes (`mirror` / `query` / `static` / `drop`); see
`doc/01_GENERAL_SPECIFICATIONS.md §4`, `doc/features/05_hierarchies.md`, and
`doc/features/18_hierarchy_improvements.md` for the full model.

Write operations (move/copy/upload/delete) ship with WebDAV and are **not** part of this API yet;
the `config` already declares the write-back model so no schema change is needed when WebDAV lands.

#### `config` shape

```ts
interface HierarchyConfig {
    version: number;                              // schema version (currently 2; v1 blobs read forward)
    safeDeleteMode: "singleBranch" | "fullDelete"; // hierarchy default
    naming: "original" | "date" | "id";           // hierarchy default (WebDAV file naming)
    writeBack: boolean;                            // master switch (hard ceiling); false ⇒ read-only
    nodes: Node[];                                 // ordered root-level tree
}

// Common node fields (incl. writeBackEnabled tri-state) + a kind discriminator.
// writeBackEnabled: true | false | null (null = inherit nearest explicit ancestor, root seed = master).
type Node =
    | { id: string; name?: string; naming?; safeDeleteMode?; writeBackEnabled?: boolean | null;
        kind: "mirror"; tagRoot: string; keepDir?: boolean;
        collapsed?: string[];       // must be <@ tagRoot
        exclude?: string[];         // may be foreign to tagRoot (pure picture-membership cut)
        maxDepth?: number;          // 0/absent = unrestricted; cap N levels below tagRoot
        deeperMode?: "collapse" | "exclude"; } // pictures below the cut (default collapse)
    | { id: string; name: string; naming?; safeDeleteMode?; writeBackEnabled?: boolean | null;
        kind: "query"; match?: "all" | "any"; include?: string[]; exclude?: string[];
        matchUntagged?: boolean; writeBack?: WriteBack | null; children?: Node[]; }
    | { id: string; name: string; naming?; safeDeleteMode?; writeBackEnabled?: boolean | null;
        kind: "static"; children?: Node[]; }
    | { id: string; name: string; naming?; safeDeleteMode?; writeBackEnabled?: boolean | null;
        kind: "drop"; onAdd: Array<{ op: "assign" | "remove"; path: string }>; };

interface WriteBack {
    onAdd: Array<{ op: "assign" | "remove"; path: string }>;
    onRemove: Array<{ op: "assign" | "remove"; path: string }>;
}
```

- **`mirror`** — expands the live tag subtree under `tagRoot`. `keepDir` keeps the `tagRoot` label as
  a directory level; `collapsed` subtrees roll their pictures up to the nearest enabled ancestor;
  a **sub-tag** `exclude` removes the subtree entirely, a **foreign** `exclude` (not under `tagRoot`)
  just rejects pictures carrying it (no directory effect). `maxDepth` caps directory generation and
  `deeperMode` (`collapse`|`exclude`) governs pictures below the cut. Leaf in the authored JSON.
- **`query`** — explicit predicate; may nest. Effective predicate = own ∧ all ancestors. `match`
  combines `include` (AND/OR); `exclude` rejects; `matchUntagged: true` means "no stored tag of any
  source" (requires empty `include`/`exclude`, but **may** now carry a free-form `writeBack`).
  `writeBack: null` ⇒ read-only directory.
- **`static`** — pure container, no predicate, no direct pictures, read-only (its `writeBackEnabled`
  still seeds descendants' inherited default).
- **`drop`** — write-only inbox (feature 18): always shown, lists nothing, applies the fixed `onAdd`
  to every upload; always writable (ignores `writeBackEnabled` and the master switch). In `tree`,
  a drop dir is always shown with `child_count: 0`, `picture_count: 0` (when `counts=true`), and
  `writable: true`; `browse` returns an empty page.

Validation (server-side, on create/update) rejects: duplicate node ids, duplicate sibling names,
`collapsed` not under `tagRoot`, `matchUntagged` with a non-empty `include`/`exclude`, and a
(non-untagged) `writeBack` op-list that cannot satisfy/break the predicate. Foreign `exclude`
entries, `drop.onAdd`, and untagged `writeBack` op-lists only need to parse as tag paths.

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

### 6.11 Batch operations (feature 14)

Every batch endpoint speaks one **selection descriptor**: a `query` (a homogenized
[`PictureFilter`](#picturefilter)) plus explicit id deltas. The effective set is
`(resolve(query) ∪ include_ids) \ exclude_ids`, always scoped server-side to the caller. Resolution
runs at **apply time** (`Ctrl+A` = "everything this query matches now"); the dry-run re-resolves
through the same path so the previewed count cannot diverge from the apply. See
`doc/features/14_better_batch_editing.md`.

```ts
interface PictureSelection {
  query?: PictureFilter | null;   // null ⇒ pure explicit set
  include_ids?: string[];         // pictures explicitly added
  exclude_ids?: string[];         // pictures subtracted from the query result
}
```

<a id="picturefilter"></a>

```ts
type PictureFilter =
  | { kind: "flat"; include_tags?: string[]; exclude_tags?: string[];
      exact?: string[];  // exactly-matched paths (strict tag nav)
      match?: "all" | "any"; untagged?: boolean;
      // shared scope/date params:
      owned_only?: boolean; shared_with_me?: boolean; include_deleted?: boolean;
      captured_after?: string; captured_before?: string }
  | { kind: "hierarchy"; hierarchy_id: string; path: string;
      /* + the same shared scope/date params */ };
```

The `flat` form mirrors `GET /pictures` (tag lists as arrays here, not comma strings). The
`hierarchy` form resolves the directory `path` to its "most-specific node wins" direct predicate,
AND-ed with the scope/date params.

Endpoints accepting a selection (all also accept a legacy `picture_ids` array as a pure explicit
set, and `dry_run: true`): `PATCH /tags`, `PATCH /pictures/exif`, `POST /pictures/trash`,
`POST /pictures/restore`, `POST /pictures/aggregate`.

**Dry-run breakdown** (returned by every batch write when `dry_run: true`, §6.1):

```ts
{
  affected: number;
  // EXIF batch only:
  edited?: number; suggested?: number; local_override?: number; unsupported?: number;
  // tags batch only:
  added?: number; removed?: number;
}
```

#### `POST /api/authenticated/pictures/aggregate`

Type-aware aggregation over a selection (§4) — a server-side GROUP BY / conditional aggregate, so a
select-all of 10k pictures is never materialised or downloaded.

**Request:**

```ts
{
  selection: PictureSelection;
  sections?: Array<"summary" | "tags" | "exif">;   // default ["summary"]
  tag_provenance?: boolean;                          // default false; only meaningful with "tags"
}
```

**Response `200`:**

```ts
{
  // summary — always returned (all from the pictures row, zero joins)
  count: number; owned_count: number; received_count: number;
  total_file_size: number; trashed_count: number; owner_deleting_count: number;
  thumbnail_pending_count: number; duplicate_count: number;
  owners: Array<{ username: string; instance: string; count: number }>;  // distinct remote owners
  exif_sync: Record<ExifSyncStatus, number>;        // histogram incl. pending_job_creation

  // tags — only when "tags" requested; ancestor-inclusive counts
  tags?: Array<{
    path: string; count: number; manual_count: number;
    sources?: Array<{ source: TagSource; count: number }>;  // only when tag_provenance=true
  }>;

  // exif — only when "exif" requested; per-field, type-aware
  exif?: Record<string, FieldAggregate>;
}

type FieldAggregate =
  | { type: "distinct"; common: unknown | null; distinct: Array<{ value: unknown; count: number }>;
      distinct_overflow: number; null_count: number }
  | { type: "numeric"; min: number | null; max: number | null; avg: number | null; null_count: number }
  | { type: "date";    min: string | null; max: string | null; avg: string | null; null_count: number }
  | { type: "gps";     bbox: { lat_min; lat_max; lng_min; lng_max } | null;
      centroid: { lat; lng } | null; null_count: number };
```

`count == total` ⇒ the tag is on every selected picture; `count < total` ⇒ on some. `manual_count`
drives the remove affordance (batch remove only deletes `manual` rows).

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

### `POST /api/admin/pictures/regenerate-thumbnails`

Bulk (re)enqueue `gen_thumbnail` jobs — which also (re)compute the metadata-stripped `content_hash`
(feature 11). Useful to repair pictures whose thumbnail job failed, or to backfill `content_hash`
across the library.

**Request:**

```ts
{
  scope?: "missing" | "all";   // default "missing"
  reextract_exif?: boolean;    // default false
  limit?: number;              // 1–100000, default 10000
}
```

- `scope: "missing"` — owned pictures with a **thumbnailable** MIME, no thumbnail, older than
  30 minutes (failed/never-run jobs). Non-thumbnailable formats are skipped so they aren't
  re-enqueued forever.
- `scope: "all"` — every owned picture (e.g. to recompute `content_hash` library-wide).
- `reextract_exif: true` also re-extracts EXIF from the file (`is_initial`); the default recomputes
  thumbnails/hashes/`content_hash` only, leaving stored EXIF untouched.

Pictures with an in-flight `gen_thumbnail` job are skipped. Received pictures are never included.

**Response `200`:** `{ enqueued: number }`

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

| Method | Path                                    | Description                                                    |
|--------|-----------------------------------------|----------------------------------------------------------------|
| `POST` | `/api/federation/auth/request`          | Request a federation JWT from another instance                 |
| `POST` | `/api/federation/auth/grant`            | Receive a federation JWT                                       |
| `POST` | `/api/federation/shares/announce`       | Announce a new share                                           |
| `POST` | `/api/federation/shares/accept`         | Notify sender of share acceptance                              |
| `POST` | `/api/federation/shares/reject`         | Notify sender of share rejection                               |
| `POST` | `/api/federation/shares/revoke`         | Revoke a share (sender → recipient)                            |
| `POST` | `/api/federation/pictures/announce`     | Deliver picture announcements for an active share              |
| `POST` | `/api/federation/pictures/unannounce`   | Remove specific pictures from a share                          |
| `POST` | `/api/federation/pictures/edit_request` | Recipient → owner: propose an EXIF edit the owner auto-applies |
| `POST` | `/api/federation/pictures/presign`      | Get presigned URLs using per-picture tokens (no JWT required)  |

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
        | "synced"               // DB and file are in sync
        | "pending"              // edit_picture job is in flight reconciling the file
        | "unsupported"          // format cannot embed EXIF; DB is updated, file is not
        | "pending_job_creation";// batch edit applied set-based; the drain will create the reconcile job (feature 14 §5)

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

**Tag paths** — all requests and responses use wire form (`Photos.Travel.Alps`). Display form is `/Photos/Travel/Alps`. Convert via
`src/lib/utils.ts:TagPath`.

**Picture orientation** — thumbnails/originals are raw pixels; the client rotates at display time from the `orientation` field (1–8). Rotating is a
normal EXIF edit (`set: { orientation }`).

**Presigned URLs** — valid ~15 minutes; cache with `staleTime ≤ 10 min`. Use the `thumbnail` query param on `GET /pictures` to embed URLs in list
items and avoid per-card round-trips.

**Pipeline wakeup** — these mutations wake the tagging pipeline asynchronously: `POST /uploads/{id}/complete`, `PATCH /tags`,
`POST /tags/rename`, `PATCH /tagging-services/{id}`, `POST /tagging-services`, `DELETE /tagging-services/{id}`. Tags converge in the background; the
frontend does not need to poll.

**EXIF sync polling** — after `POST /pictures/{id}/edit`, if `exif_sync_status = "pending"`, poll `GET /jobs/{job_id}` until `completed` or `failed`.
Use exponential backoff (1s, 2s, 4s, …, stop ~30s).

**Received pictures** — `owned = false` indicates a received picture; `owner_username`/`owner_instance` identify the true owner.
`GET /pictures/{id}/url` handles cross-instance presigning transparently. Received pictures cannot have EXIF edited.

**Share workflow** — outgoing share: `pending` → recipient accepts → `pending_first_announcement` → pipeline delivers → `active`. ShareBack: pass
`shareback_of = <incoming_share_id>`; if `allow_share_back = true` on the original share, the new share auto-activates and a `SharedTagMappingService`
rule is created automatically.
