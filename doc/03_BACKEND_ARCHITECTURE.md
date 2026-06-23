# Backend Architecture

## A) Technology stack

- **Axum** (HTTP framework) + **Tokio** (async runtime)
- **SQLx** — compile-time checked SQL, Postgres features (LTREE, JSONB, custom types), migrations
- **Redis** — session cache, presigned URL cache, federation token cache, backend domain cache

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
- Multi-step workflows run in an explicit SQL transaction. For cross-instance share creation, the outbound federation call runs inside the transaction
  so failure auto-rolls back the `OutgoingShare` insert.
- API handlers call repositories directly only for single-step CRUD with no orchestration.

## C) Module layout (`back/src/`)

```
main.rs / state.rs

domain/
  auth.rs           # TokenType, JwtClaims
  user.rs / user_settings.rs
  picture.rs        # Picture (exif_data is CameraExif; Picture::full_exif() → FullExif), PictureVersion, UploadSession
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
    shares.rs       # announce_share, send_share_accept, send_share_reject, send_revocation, announce_pictures, presign_remote_pictures, send_picture_edit_request
  resolver.rs       # self_register, update_mapping, verify_token

services/
  auth.rs / users.rs / pictures.rs / user_settings.rs / jobs.rs
  selection.rs      # feature 14: PictureSelection/PictureFilter → ResolvedSelection (membership term)
  aggregate.rs      # feature 14: type-aware summary/tags/exif aggregation + dry-run shape
  hierarchy.rs      # read resolver (build_tree, predicate_for_path / most-specific-wins) + CRUD orchestration; load_resolved + WebDAV token mgmt
  vfs.rs            # protocol-agnostic VirtualFs over the hierarchy resolver (list/stat/read + write-back);
  webdav.rs         # WebDAV Basic-auth resolution (token → session) + Redis cache
  shares/
    lifecycle.rs    # create/accept/revoke/reject + cleanup_incoming_share
    registration.rs # recipient-side received-picture register / unregister
    shareback.rs    # ShareBack auto-accept (mapping wiring)
    delivery.rs     # best-effort task delivery of the revocation-cascade unannounce
  federation.rs     # inbound federation protocol handlers

api/
  middleware/auth_user.rs / auth_admin.rs / auth_resolver.rs / auth_federation.rs / auth_worker.rs
  user/auth.rs / users.rs / pictures.rs / settings.rs / shares.rs / tags.rs / jobs.rs / tagging_services.rs / hierarchies.rs
  admin/handlers.rs + models.rs
  federation/handlers.rs + models.rs
  resolver/handlers.rs + models.rs
  worker/handlers.rs + models.rs
  webdav.rs         # WebDAV handler (OPTIONS/PROPFIND/GET/HEAD/PUT/DELETE/MOVE/COPY/MKCOL/PROPPATCH/LOCK); mounted at /webdav/{slug}

infra/
  config.rs / error.rs / redis.rs / crypto.rs / db.rs / s3.rs
  tasks.rs           # in-process Tokio task queue (tag rename, revocation-cascade unannounce)
  scheduler.rs       # RecurringTask trait + Scheduler: runs all periodic loops
  pipeline.rs        # tagging pipeline: event-driven loop + PipelineRecoverySweepTask (poll fallback)
  exif_drain.rs      # feature 14: deferred-EXIF-job drain (event-driven loop + ExifDrainWaker, poll fallback)
  pipeline/
    evaluation.rs    # per-user tag service evaluation + reconciliation, then announcement
    announcement.rs  # inline reconcile_share: PFA/errored full pass + active dirty-delta (deliver-then-record)
  job_watchdog.rs    # JobWatchdogTask (reset stale jobs) + JobCleanupTask (prune terminal jobs)
  purge_sweep.rs     # PurgeSweepTask (RecurringTask): physically purge owned, retention-expired
                     # trashed pictures — unannounce + delete tracking, S3 cleanup, hard-delete
```

## D) AppState

```rust
pub struct AppState {
    pub config: Config,
    pub db: PgPool,
    pub redis: RedisClient,
    pub jwt: JwtService,
   pub worker_jwt: JwtService,
    pub storage: StorageClient,
    pub federation: FederationClient,
    pub resolver: ResolverClient,
   pub task_queue: TaskQueue,
   pub pipeline_waker: PipelineWaker,
}
```

## E) Tagging pipeline

The pipeline runs as a background Tokio task (`infra/pipeline.rs`), evaluating enabled tagging services against dirty pictures and reconciling tag
assignments.

**Dirty picture detection** — `pictures.last_pipeline_run_at IS NULL` on new/invalidated pictures; `tagging_services.last_invalidated_at` bumps on
config changes. Dirty = `last_pipeline_run_at IS NULL OR last_pipeline_run_at < last_invalidated_at` for any enabled service.

**Wake model** — a per-user `mpsc<(Uuid, debounce)>` (`PipelineWaker`) for event-driven wakes, bounded concurrency (`PIPELINE_CONCURRENCY`, default
4),
serial per user, plus a poll fallback (`PIPELINE_POLL_INTERVAL_SECS`, default 1 hour). Woken after: ingest, manual tag edit, service config change,
inbound share announcement, `cleanup_incoming_share`. Interactive wakes (`wake`) start a run promptly; worker-driven wakes that arrive one-per-picture
(EXIF/visual reconcile completion, thumbnail completion) use `wake_debounced`, coalescing a burst into a single run over a `PIPELINE_DEBOUNCE_MS`
(default 5000) window. The window starts on the first debounced wake and is **not** reset (latency bounded to the window); an interactive wake
arriving
mid-window promotes the run to start immediately. `PIPELINE_DEBOUNCE_MS=0` disables debouncing (used by tests).

**Re-announce on worker completion** — a `gen_thumbnail` completion usually first computes `file_hash`/`blurhash`/`thumbnails_generated_at`, which may
post-date a picture's first announce. If the picture is in the `share_announcements` tracking table, completion re-marks it dirty (debounced wake) so
the announcement delta re-delivers the refreshed metadata. The race-free backstop: the recovery sweep also re-dirties any tracking row whose
`announced_updated_at` trails the picture's `updated_at`.

**Evaluation order** — `SharedTagMapping` always first. Rule and Segmentation services in user-defined `position` order. Gating accumulates tags from
`manual` + `incoming_share` + earlier services; pipeline tags re-derived from scratch each run.

**Rule predicates** — a structured JSONB predicate tree (feature 13): logical `and`/`or`/`not`
composition over spatial nodes (`gps_bbox`, `gps_radius`) and typed field-condition leaves covering
all EXIF/file/ownership attributes. Parsed into `domain::pipeline::Predicate` (validated on
create/update, evaluated against the `PipelineInput` projection). See
`doc/features/13_better_rules.md`.

**Tag storage (per-source)** — two partial unique indexes: `(picture_id, tag_path) WHERE source='manual'` and
`(picture_id, tag_path, source, source_id) WHERE source<>'manual'`. `source_id` is the `tagging_services.id` for pipeline sources,
`incoming_shares.id` for `incoming_share`, or `NULL` for `manual`.

**Reconciliation** — `PipelineRepository::reconcile_pipeline_tags` (atomic CTE per picture) inserts produced tags and deletes stale `rule`/`segment`/
`share_mapping` rows. `manual` and `incoming_share` tags are never touched.

**Announcement** — the pipeline is the single picture-announcement path, delivering inline (deliver-then-record). Handles
`pending_first_announcement` (full-coverage initial pass), `active` dirty-delta reconciliation, and failure recovery (`errored` → full reconcile after
`PIPELINE_RETRY_BACKOFF_SECS` backoff). Same-backend vs cross-instance decided by `find_local_user_id`. See
`doc/features/02_pipeline_announcement_robustness.md`.

**Owner-authoritative lifecycle + EXIF (09)** — share coverage **includes** owner-trashed-pending owned
pictures (the coverage query never excluded `deleted_at`) and is decoupled from a relayer's local
`deleted_at`. `AnnouncedPicture` carries `owner_deleted_at`/`owner_purge_at` (owned: `deleted_at` and
the derived `deleted_at + trash_retention_days`; received: the stored values) and the **owner** EXIF
snapshot (owned: the row's columns; received: `remote_exif_data`, never the relayer's merged
`exif_data`). On the recipient side, `create_received` writes `remote_exif_data` + lifecycle and
preserves `local_exif_overrides`; `exif_data` (+ the promoted `captured_at`/`gps_*`/`orientation`
columns) is re-materialised as `merge(remote_exif_data, local_exif_overrides)` (override wins
per-field). A local override is DB-only (a `metadata` event, no `edit_picture` job). The purge sweep
physically deletes owned pictures past their (derived) retention, unannouncing them. See
`doc/features/09_trash_and_exif_overrides.md`.

**Recipient EXIF editing (10)** — a per-share `allow_exif_edit` grant (on `OutgoingShare`, propagated
to the `IncomingShare`) lets a recipient propose EXIF edits. `POST /pictures/{id}/exif` on a received
picture takes `mode: "local" | "propose"`: `local` is the 09 DB-only override; `propose` requires the
grant (else 403) and sends the delta to the owner via the `pictures/edit_request` federation verb
(same-backend owners short-circuit to a direct service call). The owner re-verifies the grant
(active `OutgoingShare` to the requester with `allow_exif_edit` covering the picture — never trusts
the wire), then applies it through its existing `edit_picture` write-through, which re-announces to
all recipients. Escalating a field to a proposal clears its local override. See
`doc/features/10_recipient_exif_editing.md`.

**Service lifecycle** — **disabling** removes a service's tags; **deleting** either promotes them to `manual` (`promote_service_tags_to_manual`) or
removes them, controlled by the `promote_tags` flag.

**Batch editing & deferred EXIF jobs (14)** — every batch endpoint (`aggregate`, tags, EXIF,
trash/restore) resolves a `PictureSelection` (`services::selection`) into a `ResolvedSelection` whose
membership term (`PictureRepository::push_selection_where`) is reused as a SQL subquery — aggregation
(`services::aggregate`) and the batch writes are set-based, never materialising a 10k selection.
A batch EXIF edit cannot create one `edit_picture` job per picture synchronously: owned pictures take
a single set-based `UPDATE` that stamps `exif_sync_status = 'pending_job_creation'`, and the
deferred-job drain (`infra::exif_drain`, mirroring the pipeline's dirty-then-drain + waker + poll
fallback) creates the reconcile jobs and flips them to `pending`. Received pictures take the
set-based local-override merge (or a propose-to-owner edit in `suggest` mode). Convergence is tracked
through the `exif_sync` histogram, not per-picture job ids. See `doc/features/14_better_batch_editing.md`.

## F) API Conventions

See [`06_API_REFERENCE.md`](06_API_REFERENCE.md) for the complete endpoint catalog.

### Route groups

| Section                      | Base path                      | Auth                             |
|------------------------------|--------------------------------|----------------------------------|
| Resolver endpoints           | `/api/resolver/*`              | Resolver JWT                     |
| Admin endpoints              | `/api/admin/*`                 | User JWT with `is_admin`         |
| Public/auth endpoints        | `/api/auth/*`, `/api/public/*` | Mixed                            |
| Authenticated user endpoints | `/api/authenticated/*`         | User JWT                         |
| Federation endpoints         | `/api/federation/*`            | Federation JWT (pairwise)        |
| Worker endpoints             | `/api/worker/*`                | Worker JWT                       |
| WebDAV endpoints             | `/webdav/{slug}/*`             | Per-hierarchy token (HTTP Basic) |

### Domain terminology

| Term               | Env var         | Example                | Description                                                                                                     |
|--------------------|-----------------|------------------------|-----------------------------------------------------------------------------------------------------------------|
| **Global domain**  | `GLOBAL_DOMAIN` | `example.com`          | Public identity domain. Used in `@user:example.com`, JWTs, DB, federation. Never changes from user perspective. |
| **Backend domain** | `BACK_DOMAIN`   | `backend1.example.com` | Actual API server. Resolved via WebFinger, cached in Redis. Never stored persistently.                          |

All persistent storage uses the **global domain**. Backend domains are derived on demand and cached.

### JWT tokens

| Claim        | Description                                                                        |
|--------------|------------------------------------------------------------------------------------|
| `sub`        | Username (user tokens), global domain (federation), or worker_id (worker tokens).  |
| `uid`        | User UUID (user tokens only).                                                      |
| `is_admin`   | Boolean. Admin endpoints check this claim — there is no separate admin token type. |
| `instance`   | Global domain of the issuing instance.                                             |
| `token_type` | `user` \| `resolver` \| `federation` \| `worker`.                                  |
| `aud`        | Backend domain of the verifying instance (checked against `BACK_DOMAIN`).          |

Worker tokens: `sub = worker_id`, signed with `WORKER_JWT_SECRET` (HS256, 300 s TTL). Workers cache and refresh 30 s before expiry.

### Federation authentication (pairwise JWT)

The recipient instance issues a JWT to the requesting instance. All domains in federation messages are global domains.

1. A → B: `POST /api/federation/auth/request` `{ requester_instance, username, scope, nonce }`
2. B resolves A's backend via WebFinger; sends grant to resolved address.
3. B → A: `POST /api/federation/auth/grant` `{ issuer_instance, token, expires_at, scope, nonce }`
4. A stores token in Redis under `federation:token:{B_global_domain}`.

## G) Key Flows

### Federation consistency rules

All federation code follows one rule and three options.

**Rule — federation calls run inside the requester's transaction.** Delivery failure rolls back local changes (e.g. `create_outgoing_share` announces
inside the transaction; failure rolls back the `OutgoingShare` insert).

When a federation **handler** must itself make a federation call:

1. **Inline, same transaction** — only when the inner call does not depend on the outer uncommitted state.
2. **Return a value instead of calling back** — when the inner call would depend on uncommitted state. *ShareBack: `shares/announce`
   returns `auto_accepted: true`; the initiator acts within its own still-open transaction.*
3. **Deferred task** — when neither fits; tolerate silent failure. Used for downstream `pictures/unannounce` cascade in revocation.

**Picture announcement is pipeline-driven.** No request handler announces pictures synchronously. Accepting a share moves `OutgoingShare` to
`pending_first_announcement`; the pipeline reconciles and delivers inline.

**Revocation is local-first** (intentional exception). Local state and presign tokens are deleted immediately; downstream delivery is best-effort.

### Federation share announce

1. Alice creates `OutgoingShare` (`status = pending`). The insert and federation delivery run in a single transaction.
   - **Same-backend**: `IncomingShare` created in the same transaction; no HTTP federation.
   - **Cross-instance**: federation handshake (or cached JWT), then `POST /api/federation/shares/announce` to Bob's backend.
2. Bob accepts via `POST /api/authenticated/shares/incoming/{id}/accept`. Bob transitions `IncomingShare` to `active`, then signals the sender.
   - **Same-backend**: Alice's `OutgoingShare` → `pending_first_announcement`; pipeline takes over.
   - **Cross-instance**: `POST /api/federation/shares/accept`. On receipt Alice moves her `OutgoingShare` to `pending_first_announcement`.
3. The **pipeline** sees `pending_first_announcement`, reconciles coverage, mints per-picture tokens, delivers `pictures/announce` inline, records
   tracking rows, flips to `active`. Failure → `errored` with retry backoff.
4. Bob's `announce_pictures` handler registers each picture and assigns `/SharedToMe/...` tags (`source = incoming_share`). Only accepts `active`
   shares.
5. When Bob accesses a picture: same-backend owner → derive S3 key locally; cross-instance owner → use the picture's `picture_token` to call
   `POST /api/federation/pictures/presign` on Alice's backend.

**ShareBack** (`shares/outgoing` with `shareback_of` set, `allowShareBack = true`): the recipient auto-accepts in `shares/announce` (sets
`IncomingShare = active`, creates `SharedTagMappingService`) and returns `auto_accepted: true` instead of calling back (rule 2). The initiator moves
its `OutgoingShare` to `pending_first_announcement`.

### Federation share revocation

1. Alice calls `POST /api/authenticated/shares/outgoing/{id}/revoke`.
   - **Same-backend**: removes `/SharedToMe/…` tags, deletes unreachable received pictures, sets `IncomingShare` to `revoked`, invalidates Redis
     presign-token cache.
   - **Cross-instance**: `POST /api/federation/shares/revoke` → same cleanup on Bob's backend.
2. Bob's backend propagates revocation downstream to any transitive recipients.
