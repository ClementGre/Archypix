# Backend Architecture

## A) Technology stack

- **Axum** (HTTP framework) + **Tokio** (async runtime) — consistent with the resolver component.
- **SQLx** — compile-time checked SQL, direct Postgres feature access (LTREE, JSONB, custom types), migration support.
- **Redis** — session cache, presigned URL cache, federation token cache, backend domain cache.

## B) Layered architecture

| Layer        | Responsibility                                                                | Can depend on                               | Must NOT depend on                |
|--------------|-------------------------------------------------------------------------------|---------------------------------------------|-----------------------------------|
| `api`        | HTTP handlers, auth extraction, request/response models.                      | `services`, `repository`, `domain`, `infra` | External connectivity details.    |
| `services`   | Multi-step workflows and transaction boundaries.                              | `repository`, `clients`, `domain`, `infra`  | Axum types, HTTP-specific models. |
| `clients`    | Outbound HTTP adapters (federation backends, resolver, S3).                   | `infra`, `domain`                           | `services`, `repository`, `api`.  |
| `repository` | SQL operations only — no business logic.                                      | `domain`, `infra::error`                    | `services`, `clients`.            |
| `domain`     | Business types, invariants, pure transformations, tagging pipeline evaluator. | std + lightweight crates only               | `repository`, `infra`, clients.   |
| `infra`      | Raw connectivity primitives: config, error, Redis, S3, crypto (JWT, hashing). | External SDKs                               | `api`, `services`, `clients`.     |
| `state`      | `AppState` — bootstrap, holds all composed handles.                           | `infra`, `clients`                          | `services`, `repository`, `api`.  |

**Key rules:**

- Repository functions accept `Executor<'e, Database = Postgres>` — callable on pool or transaction.
- Multi-step workflows (user creation, picture upload, share creation, job completion) run in an explicit SQL transaction managed by the service or
  handler. For cross-instance share creation, the outbound federation HTTP calls run while the transaction is still open so that any federation
  failure
  automatically rolls back the `OutgoingShare` insert.
- API handlers call repositories directly only for single-step CRUD with no orchestration.

## C) Module layout (`back/src/`)

```
main.rs / state.rs

domain/
  auth.rs           # TokenType, JwtClaims
  user.rs / user_settings.rs
  picture.rs        # Picture (includes file_hash, file_size), PictureVersion, UploadSession
  tag.rs            # TagPath (newtype), TagSource, Tag
  hierarchy.rs      # HierarchyConfig + Node tree (mirror/query/static), validation, TagPredicate
  share.rs          # OutgoingShare, IncomingShare
  federation.rs     # FederationMessage, BackendMapping
  job.rs            # Job (includes claim_token), re-exports from archypix-common
  tagging.rs / pipeline.rs   # pipeline config types + pure evaluator

repository/
  user.rs / picture.rs / picture_version.rs / user_settings.rs
  tag.rs          # per-source tag CRUD, service-tag promotion/removal helpers
  picture.rs      # picture CRUD + list/count; push_filters renders TagPredicate + legacy `tag`
  hierarchy.rs    # hierarchy CRUD SQL (load/store config JSONB)
  share.rs / auth.rs / job.rs / tagging.rs
  pipeline.rs     # dirty-picture queries, atomic per-source pipeline tag reconcile

clients/
  federation/
    mod.rs          # FederationClient struct + shared protocol types
    handshake.rs    # WebFinger resolution, token request/grant/store/issue
    shares.rs       # announce_share, send_share_accept, send_share_reject, send_revocation, announce_pictures, presign_remote_pictures
  resolver.rs       # self_register, update_mapping, verify_token

services/
  auth.rs / users.rs / pictures.rs / user_settings.rs / jobs.rs
  hierarchy.rs      # read resolver (build_tree, predicate_for_path / most-specific-wins) + CRUD orchestration
  shares.rs         # module root re-exporting the submodules below
  shares/
    lifecycle.rs    # create/accept/revoke/reject + cleanup_incoming_share (share state)
    registration.rs # recipient-side received-picture register / unregister
    shareback.rs    # ShareBack auto-accept (mapping wiring)
    delivery.rs     # best-effort task delivery of the revocation-cascade unannounce
  federation.rs     # inbound federation protocol handlers (receive_share_announcement, receive_share_accept, receive_share_revoke, receive_share_reject, receive_pictures_announcement, receive_pictures_unannouncement, presign_by_picture_tokens)

api/
  middleware/auth_user.rs / auth_admin.rs / auth_resolver.rs / auth_federation.rs / auth_worker.rs
  user/auth.rs / users.rs / pictures.rs / settings.rs / shares.rs / tags.rs / jobs.rs / tagging_services.rs / hierarchies.rs
  admin/handlers.rs + models.rs
  federation/handlers.rs + models.rs
  resolver/handlers.rs + models.rs
  worker/handlers.rs + models.rs

infra/
  config.rs / error.rs / redis.rs / crypto.rs / db.rs / s3.rs
  tasks.rs           # in-process Tokio task queue (tag rename, revocation-cascade unannounce)
  scheduler.rs       # RecurringTask trait + Scheduler: runs all periodic loops (one spawned loop each)
  pipeline.rs        # tagging pipeline: event-driven loop + PipelineRecoverySweepTask (poll fallback)
  pipeline/
    evaluation.rs    # per-user tag service evaluation + reconciliation, then announcement
    announcement.rs  # inline reconcile_share: PFA/errored full pass + active dirty-delta (deliver-then-record);
                     #   re-announces tracked pictures whose metadata changed (gated on pictures.updated_at)
  job_watchdog.rs    # JobWatchdogTask (reset stale processing jobs) + JobCleanupTask (prune terminal jobs)
```

## D) AppState

```rust
pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    pub redis: RedisClient,
    pub jwt: JwtService,
    pub worker_jwt: JwtService,      // verifies inbound worker tokens
    pub storage: StorageClient,
    pub federation: FederationClient,
    pub resolver: ResolverClient,
    pub task_queue: TaskQueue,       // in-process task queue (tag rename)
    pub pipeline_waker: PipelineWaker, // per-user wake handle (mpsc<Uuid>) for the pipeline loop
}
```

## E) Tagging pipeline

The pipeline runs as a background Tokio task (`infra/pipeline.rs`). It evaluates enabled tagging services against dirty pictures and reconciles tag
assignments.

**Dirty picture detection** — `pictures.last_pipeline_run_at` is `NULL` on new/invalidated pictures; `tagging_services.last_invalidated_at` bumps on
any config change. A picture is dirty when `last_pipeline_run_at IS NULL OR last_pipeline_run_at < last_invalidated_at` for any enabled service.

**Wake model** — a per-user `mpsc<Uuid>` (`PipelineWaker`) for event-driven wakes, consumed by a per-user coalescing scheduler that runs users
concurrently (bounded by `PIPELINE_CONCURRENCY`, default 4) and serially per user, plus a configurable polling fallback (
`PIPELINE_POLL_INTERVAL_SECS`, default 1 hour) that re-enqueues all dirty users. A missed wake is latency-only — the poll sweep recovers it. See
`doc/features/02_pipeline_announcement_robustness.md` §7.
Notified after: ingest, manual tag edit, service config change, inbound share announcement, `cleanup_incoming_share`.

**Evaluation order** — `SharedTagMapping` always runs first (no user control; it only depends on incoming share IDs). Rule and Segmentation services
are then evaluated in user-defined `position` order (interleaved freely). Gating accumulates tags from `manual` + `incoming_share` plus earlier
services (in-memory, per picture); pipeline tags are re-derived from scratch each run, never carried forward.

**Rule predicates** — stored in `rule_tagging_services.predicate`, validated at creation. Supported:
`gps_within_bbox(lat_min, lat_max, lon_min, lon_max)`, `capture_year(YYYY)`, `capture_month(M)`, `filename_contains("string")`.

**Tag storage (per-source)** — the same `tag_path` may be asserted by multiple sources on one picture. Two partial unique indexes:
`(picture_id, tag_path) WHERE source='manual'` and `(picture_id, tag_path, source, source_id) WHERE source<>'manual'`. `source_id` is the
`tagging_services.id` for pipeline sources, `incoming_shares.id` for `incoming_share`, or `NULL` for `manual`.

**Reconciliation** — `PipelineRepository::reconcile_pipeline_tags` (atomic CTE per picture) inserts produced tags and deletes stale `rule`/`segment`/
`share_mapping` rows. `manual` and `incoming_share` tags are never touched.

**Announcement** — the pipeline is the single picture-announcement path and delivers **inline**
(deliver-then-record: federation call / same-backend registration first, tracking rows + status flip
only on success). Two entry points in `pipeline::announcement` wrap an internal `reconcile_share`:

- `reconcile_pending_and_errored` — for each `pending_first_announcement` (initial) or `errored`
  (failure recovery) share whose `next_retry_at` backoff has elapsed: diffs **full** coverage against
  the tracking table, delivers, and flips to `active` once fully delivered.
- `reconcile_active_batch` — after reconciliation: diffs `active` + `future=true` shares over the
  batch's **dirty** pictures, announcing new coverage, unannouncing lost coverage, re-announcing
  received pictures whose upstream token moved, and re-announcing tracked pictures whose owner
  metadata changed (gated on `pictures.updated_at > share_announcements.announced_updated_at`, so an
  EXIF edit propagates to recipients but a tag-only change does not). Announced pictures carry the
  owner's gps/orientation/exif_data so recipients converge on the same metadata.

A failed delivery demotes an `active` share to `errored` and sets a `next_retry_at` backoff
(`PIPELINE_RETRY_BACKOFF_SECS`); the next pass is then a full reconcile. Same-backend vs cross-instance
is decided by `find_local_user_id` (not a global-domain comparison). The sweep wakes for users with
dirty pictures **or** a `pending_first_announcement`/`errored` share past its backoff. See
`doc/features/02_pipeline_announcement_robustness.md`.

**Service lifecycle** — **disabling** deletes its tags (`TagRepository::remove_service_tags`); **deleting** either promotes them to `manual` (
`promote_service_tags_to_manual`, pre-existing manual tag on the same path wins) or removes them, controlled by the `promote_tags` flag.

# Backend REST API

## 1) API layout

| Section                      | Base path                      | Auth                      |
|------------------------------|--------------------------------|---------------------------|
| Resolver endpoints           | `/api/resolver/*`              | Resolver JWT              |
| Admin endpoints              | `/api/admin/*`                 | User JWT with `is_admin`  |
| Public/auth endpoints        | `/api/auth/*`, `/api/public/*` | Mixed                     |
| Authenticated user endpoints | `/api/authenticated/*`         | User JWT                  |
| Federation endpoints         | `/api/federation/*`            | Federation JWT (pairwise) |
| Worker endpoints             | `/api/worker/*`                | Worker JWT                |

## 2) Domain terminology

| Term               | Env var         | Example                | Description                                                                                                     |
|--------------------|-----------------|------------------------|-----------------------------------------------------------------------------------------------------------------|
| **Global domain**  | `GLOBAL_DOMAIN` | `example.com`          | Public identity domain. Used in `@user:example.com`, JWTs, DB, federation. Never changes from user perspective. |
| **Backend domain** | `BACK_DOMAIN`   | `backend1.example.com` | Actual API server. Resolved via WebFinger, cached in Redis. Never stored persistently.                          |

All persistent storage uses the **global domain**. Backend domains are derived on demand and cached.

## 3) JWT tokens

| Claim        | Description                                                                               |
|--------------|-------------------------------------------------------------------------------------------|
| `sub`        | Username (user tokens) or global domain (federation tokens) or worker_id (worker tokens). |
| `uid`        | User UUID (user tokens only).                                                             |
| `is_admin`   | Boolean. Admin endpoints check this, not a separate token type.                           |
| `instance`   | Global domain of the issuing instance.                                                    |
| `token_type` | `user` \| `resolver` \| `federation` \| `worker`. There is no `admin` token type.         |
| `aud`        | Backend domain of the verifying instance (checked against `BACK_DOMAIN`).                 |

Worker tokens: `sub = worker_id`, `iss = global_domain`, signed with `WORKER_JWT_SECRET` (HS256, 300 s TTL). Workers cache the token and refresh it 30
s before expiry, so at most one token generation per ~270 s per worker process.

## 4) Federation authentication (pairwise JWT)

The recipient instance issues a JWT to the requesting instance. All domains in federation messages are global domains — backend domains are resolved
via WebFinger and cached.

**Handshake:**

1. A → B: `POST /api/federation/auth/request` `{ requester_instance, username, scope, nonce }`
2. B resolves A's backend via WebFinger; sends grant to resolved address.
3. B → A: `POST /api/federation/auth/grant` `{ issuer_instance, token, expires_at, scope, nonce }`
4. A stores token in Redis under `federation:token:{B_global_domain}`.

## 5) Endpoint layout

### Resolver endpoints (on backend, called by Resolver)

| Method | Path                             | Description                                  |
|--------|----------------------------------|----------------------------------------------|
| `POST` | `/api/resolver/users`            | Create user (only when `USE_RESOLVER=true`). |
| `GET`  | `/api/resolver/users/{username}` | Fetch user for resolver validation.          |

### Resolver service endpoints (port 8080)

| Method | Path                                                    | Description                                    |
|--------|---------------------------------------------------------|------------------------------------------------|
| `GET`  | `/.well-known/webfinger?resource=archypix:@user:domain` | Resolve username to backend URL.               |
| `POST` | `/api/register`                                         | Register user; routes to least-loaded backend. |
| `POST` | `/api/backends`                                         | Backend self-registration at startup.          |
| `POST` | `/api/update`                                           | Update `username → back_domain` mapping.       |

### Admin endpoints

Auth: User JWT with `is_admin = true`.

**Instance**

| Method | Path                  | Description                                                                                         |
|--------|-----------------------|-----------------------------------------------------------------------------------------------------|
| `GET`  | `/api/admin/instance` | Instance health: global_domain, back_domain, DB/Redis connectivity, last worker activity timestamp. |

**Analytics** (responses cached in Redis; instance stats 60 s TTL, per-user 120 s TTL)

| Method | Path                          | Description                                                                                                                                     |
|--------|-------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------|
| `GET`  | `/api/admin/stats`            | Instance-wide analytics: user/picture counts, total storage, job queue depth, errored share count, dirty picture count.                         |
| `GET`  | `/api/admin/consistency`      | Consistency check: stuck EXIF-pending pictures (no active job), pictures without thumbnails (>30 min old), broken SharedTagMappingService rows. |
| `GET`  | `/api/admin/users/{id}/stats` | Per-user analytics: picture counts, storage, job counts by status, share counts by status, dirty picture count.                                 |

**User management**

| Method   | Path                                  | Description                                                                                               |
|----------|---------------------------------------|-----------------------------------------------------------------------------------------------------------|
| `GET`    | `/api/admin/users`                    | List users with storage used (bytes).                                                                     |
| `POST`   | `/api/admin/users`                    | Create user (admin override).                                                                             |
| `PATCH`  | `/api/admin/users/{id}`               | Update display name or admin role.                                                                        |
| `DELETE` | `/api/admin/users/{id}`               | Delete user.                                                                                              |
| `GET`    | `/api/admin/users/{id}/shares`        | List a user's outgoing and incoming shares with full status (useful for diagnosing errored/stuck shares). |
| `POST`   | `/api/admin/users/{id}/pipeline/wake` | Force-wake the tagging pipeline for a user immediately.                                                   |

**Job management**

| Method | Path                          | Description                                                                                    |
|--------|-------------------------------|------------------------------------------------------------------------------------------------|
| `GET`  | `/api/admin/jobs`             | List jobs. Query params: `status`, `type`, `user_id`, `limit` (max 200, default 50), `offset`. |
| `GET`  | `/api/admin/jobs/stale`       | List jobs currently in `processing` past `JOB_PROCESSING_TIMEOUT_SECS`.                        |
| `POST` | `/api/admin/jobs/{id}/reset`  | Force-reset a non-completed job to `pending` (clears claim_token, resets retry_count to 0).    |
| `POST` | `/api/admin/jobs/{id}/cancel` | Permanently fail a non-terminal job (sets status to `failed`).                                 |

**Share management**

| Method | Path                                              | Description                                                                                                              |
|--------|---------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------|
| `GET`  | `/api/admin/shares/errored`                       | List all `errored` outgoing shares across all users with `next_retry_at` and `last_error_at`.                            |
| `POST` | `/api/admin/shares/outgoing/{id}/force-reconcile` | Clear the backoff on an `errored` or `pending_first_announcement` share and immediately wake the pipeline for its owner. |

**Federation**

| Method | Path                              | Description                                                                                               |
|--------|-----------------------------------|-----------------------------------------------------------------------------------------------------------|
| `GET`  | `/api/admin/federation/instances` | List all known remote instances (derived from share records) with outgoing/incoming/errored share counts. |

### Public/auth endpoints

| Method | Path                           | Description                                     |
|--------|--------------------------------|-------------------------------------------------|
| `POST` | `/api/auth/login`              | Login (username + password).                    |
| `POST` | `/api/auth/refresh`            | Refresh access token.                           |
| `POST` | `/api/auth/logout`             | Revoke session.                                 |
| `GET`  | `/api/auth/me`                 | Current user profile (user JWT required).       |
| `GET`  | `/api/public/users/{username}` | Public profile lookup.                          |
| `POST` | `/api/public/users`            | Register user (only when `USE_RESOLVER=false`). |

### Authenticated user endpoints (`/api/authenticated/*`)

**Users & settings**

| Method  | Path                          | Description                                  |
|---------|-------------------------------|----------------------------------------------|
| `PATCH` | `/api/authenticated/users/me` | Update profile.                              |
| `GET`   | `/api/authenticated/settings` | Get user settings.                           |
| `PATCH` | `/api/authenticated/settings` | Update settings. Body: `{ versioning_mode }` |

**Pictures — upload**

| Method | Path                                                        | Description                                                                                                                                                                                                     |
|--------|-------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `POST` | `/api/authenticated/pictures/uploads`                       | Begin single upload. Returns `{ picture_id, presigned_url }` (staging bucket).                                                                                                                                  |
| `POST` | `/api/authenticated/pictures/uploads/batch`                 | Begin batch upload. Body: `{ filenames: string[] }` (max 100). Returns array of `{ picture_id, presigned_url }` in the same order. Presigns all slots in parallel.                                              |
| `POST` | `/api/authenticated/pictures/uploads/{picture_id}/complete` | Confirm upload. Optional body: `{ mime_type, file_size, width, height, initial_tags?, ... }`. Assigns `initial_tags` as manual tags, enqueues a `gen_thumbnail` job; all created atomically in one transaction. |

**Pictures — list & details**

| Method | Path                                   | Description                                                                                                                                                                                                                          |
|--------|----------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `GET`  | `/api/authenticated/pictures`          | Paginated list. Query params: `page`, `page_size`, `sort`, `order`, `tag`, `include_tags`, `exclude_tags`, `match`, `untagged`, `owned_only`, `shared_with_me`, `include_deleted`, `captured_after`, `captured_before`, `thumbnail`. |
| `GET`  | `/api/authenticated/pictures/{id}`     | Full picture details + version history.                                                                                                                                                                                              |
| `GET`  | `/api/authenticated/pictures/{id}/url` | Presigned URL for a variant. Query: `variant=original\|small\|medium\|large`.                                                                                                                                                        |

**Pictures — editing**

| Method  | Path                                           | Description                                                                                                                                                                                             |
|---------|------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `POST`  | `/api/authenticated/pictures/{id}/edit`        | Edit one owned picture's EXIF (write-through). Body: `{ set?, clear? }`. Applies the DB change synchronously; returns the updated row, `exif_sync_status`, and `job_id` (or `null` when `unsupported`). |
| `PATCH` | `/api/authenticated/pictures/exif`             | Batch EXIF edit. Body: `{ picture_ids, set?, clear? }` (owned only, no cap). Returns `{ updated, jobs, unsupported }`.                                                                                  |
| `POST`  | `/api/authenticated/pictures/{id}/exif/resync` | Re-enqueue a reconcile for a picture stuck in `pending` with no in-flight job.                                                                                                                          |
| `GET`   | `/api/authenticated/pictures/{id}/jobs`        | List all processing jobs for a picture.                                                                                                                                                                 |

**Jobs**

| Method | Path                           | Description                                           |
|--------|--------------------------------|-------------------------------------------------------|
| `GET`  | `/api/authenticated/jobs/{id}` | Get the status and result of a job (owned by caller). |

**Tags**

Tag paths are dot-separated `ltree` form (`Photos.Travel.Alps`) on the wire — the same form the API returns, so responses feed straight back into
requests. Allowed label characters are `[A-Za-z0-9_]`.

| Method  | Path                      | Description                                                                                                                                                                                                                                         |
|---------|---------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `GET`   | `/api/authenticated/tags` | List tags. No params: all distinct tag paths for the user. `?picture_id=` : that picture's tags folded to the deepest distinct paths; add `&with_sources=true` to get each path with the list of `{ source, source_id }` asserting it (provenance). |
| `PATCH` | `/api/authenticated/tags` | Batch edit tags. Body: `{ picture_ids, add_tags, remove_tags }`. Applies add/remove atomically to all listed pictures. Only `manual` tags are removed.                                                                                              |

**Tagging pipeline**

| Method   | Path                                                      | Description                                                                                                                                  |
|----------|-----------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------|
| `GET`    | `/api/authenticated/tagging-services`                     | List all tagging services with their embedded rules, ordered by pipeline execution order.                                                    |
| `POST`   | `/api/authenticated/tagging-services`                     | Create a tagging service. Body: `{ service_type, requires?, excludes? }`. New service gets `position = max(position)+1`.                     |
| `POST`   | `/api/authenticated/tagging-services/reorder`             | Set execution order of Rule and Segmentation services. Body: `{ ordered_ids: [uuid, …] }`. SharedTagMapping IDs must not be included.        |
| `GET`    | `/api/authenticated/tagging-services/{id}`                | Get a specific service with its rules.                                                                                                       |
| `PATCH`  | `/api/authenticated/tagging-services/{id}`                | Update a service. Body: `{ enabled?, requires?, excludes? }`. Omitted fields are unchanged.                                                  |
| `DELETE` | `/api/authenticated/tagging-services/{id}`                | Delete a service (cascades to all its rules). Query: `promote_tags` (required) — `true` promotes its tags to `manual`, `false` removes them. |
| `POST`   | `/api/authenticated/tagging-services/{id}/mappings`       | Add a mapping rule (shared\_tag\_mapping only). Body: `{ incoming_share_id, assign_tag }`.                                                   |
| `DELETE` | `/api/authenticated/tagging-services/{id}/mappings/{rid}` | Delete a mapping rule.                                                                                                                       |
| `POST`   | `/api/authenticated/tagging-services/{id}/rules`          | Add a predicate rule (rule type only). Body: `{ predicate, assign_tag }`.                                                                    |
| `DELETE` | `/api/authenticated/tagging-services/{id}/rules/{rid}`    | Delete a predicate rule.                                                                                                                     |
| `POST`   | `/api/authenticated/tagging-services/{id}/segments`       | Add a date-range segment (segmentation only). Body: `{ name, date_start, date_end, assign_tag, parent_segment_id? }`.                        |
| `DELETE` | `/api/authenticated/tagging-services/{id}/segments/{sid}` | Delete a segment (cascades to child segments).                                                                                               |

**Hierarchies**

A hierarchy maps a filtered view of the tag graph to a directory tree (node-tree `config` JSONB —
mirror/query/static). It stores no pictures; membership is derived live. Write endpoints ship with
WebDAV (out of scope here). See `doc/features/05_hierarchies.md`.

| Method   | Path                                         | Description                                                                                                     |
|----------|----------------------------------------------|-----------------------------------------------------------------------------------------------------------------|
| `GET`    | `/api/authenticated/hierarchies`             | List the user's hierarchies (`id`, `name`, `enabled`).                                                          |
| `POST`   | `/api/authenticated/hierarchies`             | Create. Body: `{ name, config? }`. Validates `config` (§11); stores the normalized form.                        |
| `GET`    | `/api/authenticated/hierarchies/{id}`        | Get one with full `config`.                                                                                     |
| `PATCH`  | `/api/authenticated/hierarchies/{id}`        | Update `{ name?, enabled?, config? }`. Re-validates `config`.                                                   |
| `DELETE` | `/api/authenticated/hierarchies/{id}`        | Delete.                                                                                                         |
| `GET`    | `/api/authenticated/hierarchies/{id}/tree`   | Directories only. Query: `path` (default root), `depth` (default 1), `counts`.                                  |
| `GET`    | `/api/authenticated/hierarchies/{id}/browse` | Paginated pictures of one directory. Resolves `path` → `TagPredicate` server-side, then reuses `list_pictures`. |

**Sharing**

| Method | Path                                             | Description                                                                                 |
|--------|--------------------------------------------------|---------------------------------------------------------------------------------------------|
| `POST` | `/api/authenticated/shares/outgoing`             | Create outgoing share.                                                                      |
| `GET`  | `/api/authenticated/shares/outgoing`             | List outgoing shares.                                                                       |
| `POST` | `/api/authenticated/shares/outgoing/{id}/revoke` | Revoke an outgoing share. Notifies the recipient; removes their tags and received pictures. |
| `GET`  | `/api/authenticated/shares/incoming`             | List incoming shares.                                                                       |
| `POST` | `/api/authenticated/shares/incoming/{id}/accept` | Accept incoming share (`pending → active`).                                                 |
| `POST` | `/api/authenticated/shares/incoming/{id}/reject` | Reject incoming share (`pending/active → tombstoned`).                                      |

### Federation endpoints

| Method | Path                                  | Description                                                                                                                                                  |
|--------|---------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `POST` | `/api/federation/auth/request`        | Request a federation JWT.                                                                                                                                    |
| `POST` | `/api/federation/auth/grant`          | Receive a federation JWT from another instance.                                                                                                              |
| `POST` | `/api/federation/shares/announce`     | Share announcement. Requires federation JWT.                                                                                                                 |
| `POST` | `/api/federation/shares/accept`       | Recipient notifies sender that a share was accepted. Sender responds by announcing current pictures. Requires federation JWT.                                |
| `POST` | `/api/federation/shares/revoke`       | Share revocation. Body: `{ outgoing_share_id }`. Requires federation JWT.                                                                                    |
| `POST` | `/api/federation/pictures/announce`   | Announce pictures for an active share. Only accepted when `IncomingShare.status == active`. Requires federation JWT. Each picture carries a `picture_token`. |
| `POST` | `/api/federation/pictures/unannounce` | Remove specific pictures from a share. Body: `{ outgoing_share_id, sender_username, sender_instance, picture_ids }`. Requires federation JWT.                |
| `POST` | `/api/federation/pictures/presign`    | Request presigned URLs. Auth: `picture_token` per picture — no JWT required. Body: `{ pictures: [{picture_token, variant}] }`.                               |

### Worker endpoints (`/api/worker/*`)

Auth: `Authorization: Bearer <worker_jwt>` — short-lived JWT (HS256, 300 s TTL) signed with `WORKER_JWT_SECRET` (`token_type: worker`). Workers cache
the token and refresh 30 s before expiry.

| Method | Path                             | Description                                                                                                           |
|--------|----------------------------------|-----------------------------------------------------------------------------------------------------------------------|
| `GET`  | `/api/worker/jobs/next`          | Atomically claim next pending job. Returns job + presigned S3 URLs + `claim_token`, or `null`. Query: `types=…`.      |
| `POST` | `/api/worker/jobs/{id}/complete` | Report success. Backend applies picture updates and marks job `completed` in one transaction. Requires `claim_token`. |
| `POST` | `/api/worker/jobs/{id}/fail`     | Report failure. Auto-retries up to `max_retries` (default 3) unless `permanent: true`. Requires `claim_token`.        |

Wire shapes are defined in `archypix-common/transfer.rs`. See `04_WORKER_ARCHITECTURE.md` for the claim-token protocol and job loop.

## 6) Key flows

### Federation consistency rules

All federation code follows one rule and three options.

**Rule — federation calls run inside the requester's transaction.** A delivery failure rolls back local changes (e.g. `create_outgoing_share`
announces inside the transaction that inserted the `OutgoingShare`; failure rolls it back).

When a federation **handler** must itself make a federation call, pick the first option that applies:

1. **Inline, same transaction** — only when the inner call does not depend on the outer requester's uncommitted state.
2. **Return a value instead of calling back** — when the inner call would depend on uncommitted state. *ShareBack: `shares/announce`
   returns `auto_accepted: true`; the initiator acts within its own still-open transaction instead of receiving a callback.*
3. **Deferred task** — when neither fits; tolerate silent failure since the outer request can no longer be rolled back. Used for the best-effort
   downstream `pictures/unannounce` cascade emitted by `cleanup_incoming_share` during revocation.

**Picture announcement is pipeline-driven.** No request handler announces pictures synchronously. Accepting a share moves the `OutgoingShare` to
`pending_first_announcement`; the pipeline reconciles its coverage and delivers **inline** (deliver-then-record), flipping it to `active` on success
or
to `errored` (with a retry backoff) on failure. This makes initial and ongoing announce one mechanism, eliminates cross-backend callbacks on
uncommitted state, and makes a failed delivery self-healing — the next reconcile re-derives it from coverage-vs-tracking, so nothing is silently lost.

**Revocation is local-first** (intentional exception). Local state and presign tokens are deleted immediately; downstream delivery of
`shares/revoke` / `pictures/unannounce` is best-effort.

### Picture upload

For single or batch uploads the flow is the same per file; only step 1 differs:

1. Client → `POST /uploads` (single) **or** `POST /uploads/batch` (multi) → gets `{ picture_id, presigned_url }` per file. Batch presigns all slots
   in parallel server-side.
2. Client → MinIO: `PUT` binary to presigned URL (parallel across files, recommended max 4 concurrent).
3. Client → `POST /uploads/{id}/complete` for each file **as it finishes** (not after all files) → backend: copies staging → pictures bucket (+
   versions bucket if versioning enabled); **single DB transaction** creates `pictures` row, `picture_versions` row, any `initial_tags` as manual tag
   rows, and `gen_thumbnail` job.
4. Worker claims job, processes the original (EXIF, thumbnails, BlurHash, SHA-256), and calls `POST /api/worker/jobs/{id}/complete`. Backend updates
   the picture row and marks the job done in one transaction; rejects on `claim_token` mismatch (409).

S3 keys: `{user_id}/{picture_id}` for originals/thumbnails; `{user_id}/{picture_id}/{version_id}` for versions. Keys are never stored — derived on
demand.

### Federation handshake

1. Alice’s backend resolves Bob's backend url via WebFinger.
2. Alice’s backend requests a Federation JWT to Bob’s backend at `POST /api/federation/auth/request`.
3. Bob’s backend resolves Alice’s backend via WebFinger.
4. Bob’s backend sends a JWT to Alice’s backend at `POST /api/federation/auth/grant`.

### Federation share announce

1. Alice creates `OutgoingShare` (`status = pending`). The `OutgoingShare` insert and the federation delivery run in a single transaction: if the
   federation call fails the insert is rolled back.
    - **Same-backend** (`recipient_instance == global_domain`): `IncomingShare` is created in the same transaction (`status = pending`); no HTTP
      federation.
    - **Cross-instance**: federation handshake (or JWT from cache), then `POST /api/federation/shares/announce` to Bob’s backend. Bob’s backend
      creates
      `IncomingShare` (`status = pending`).
2. Bob accepts the share via `POST /api/authenticated/shares/incoming/{id}/accept`. Bob’s backend **immediately transitions `IncomingShare`
   to `active`** (Bob’s consent), then signals the sender — but never announces pictures itself:
    - **Same-backend**: Alice’s `OutgoingShare` is moved to `pending_first_announcement`; the pipeline takes over (step 3).
    - **Cross-instance**: sends `POST /api/federation/shares/accept` to Alice’s backend. If delivery fails the `IncomingShare` reverts to `pending`.
      On receipt, Alice moves her `OutgoingShare` to `pending_first_announcement`.
3. The **pipeline** (on the sender's backend) sees the `pending_first_announcement` share and reconciles its current coverage (ignoring `future`):
   it mints a per-picture token, delivers `pictures/announce` **inline** (cross-instance HTTP, or same-backend registration), and — only on success —
   records the tracking rows and flips the share to `active`. A delivery failure moves it to `errored` with a retry backoff instead. This is the
   single
   announce path; `future = true` shares are subsequently re-diffed by the same pipeline.

**ShareBack** (`shares/outgoing` with `shareback_of` set, `allowShareBack = true` on the referenced share): the recipient *auto-accepts* in its
`shares/announce` handler (sets its `IncomingShare = active` and creates the `SharedTagMappingService` mapping) and returns `auto_accepted: true`
**instead of calling back** (rule 2). The initiator moves its own `OutgoingShare` to `pending_first_announcement` and lets its pipeline announce. If
`allowShareBack = false`, the share stays `pending` for manual acceptance.
4. Bob’s `announce_pictures` handler registers each announced picture (`PictureRepository::create_received`) and assigns the
   `/SharedToMe/alice_AT_instance_DOT_com/…` tag (`source = incoming_share`). It only accepts `active` shares — `pending` shares are rejected (
   prevents picture injection into unaccepted shares).
5. When Bob accesses a picture, `presign_for_picture` checks Redis cache first, then:
    - **Same-backend owner**: derives S3 key from `remote_picture_id` and owner’s local `user_id`.
   - **Cross-instance owner**: looks up the picture's per-picture `picture_token` (from its `incoming_share` tag row), calls
     `POST /api/federation/pictures/presign` on Alice’s backend with that token.

### Federation share revocation

1. Alice calls `POST /api/authenticated/shares/outgoing/{id}/revoke`. Alice’s backend sets `OutgoingShare` status to `revoked` and notifies the
   recipient.
    - **Same-backend**: directly removes `/SharedToMe/…` tags, deletes unreachable received pictures, sets `IncomingShare` status to `revoked`,
      invalidates Redis presign-token cache.
    - **Cross-instance**: sends `POST /api/federation/shares/revoke` (keyed by `outgoing_share_id`) to Bob’s backend, which performs the same cleanup
      locally.
2. Bob’s backend propagates revocation downstream to any transitive recipients.
