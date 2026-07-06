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
| `repository` | SQL operations only — no business logic.                                      | `domain`                                    | `services`, `clients`.            |
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
  hierarchy.rs      # HierarchyConfig + Node tree (mirror/query/static/drop), validation,
                    #   per-node writeBackEnabled (feature 18 effective_enabled), TagPredicate
  share.rs          # OutgoingShare, IncomingShare
  federation.rs     # FederationMessage, BackendMapping
  job.rs            # Job (includes claim_token), re-exports from archypix-common
  tagging.rs        # service model + ServiceConfig (parse/validate/normalize/evaluate dispatch) + should_run
  pipeline.rs       # PipelineInput (the picture projection the evaluator reads)
  predicate.rs      # feature 13: rule predicate engine (Predicate/Field/Condition + parsing)
  segmentation.rs   # feature 20: SegmentationConfig (band-list parse/validate/resolve)

repository/
  user.rs / picture.rs / picture_version.rs / user_settings.rs
  user_storage.rs # feature 22: read the trigger-maintained billed breakdown; reconcile recompute
  tag.rs          # per-source tag CRUD, service-tag promotion/removal helpers
  picture.rs      # picture CRUD + list/count; push_filters renders TagPredicate + legacy `tag`
  hierarchy.rs    # hierarchy CRUD SQL (load/store config JSONB)
  share.rs / auth.rs / job.rs / tagging.rs
  pipeline.rs     # dirty-picture queries, atomic per-source pipeline tag reconcile
  dedup.rs        # feature 11: content-dedup group queries (candidate keys, group rows, survivor/
                  # promote/boomerang mutations)

clients/
  federation/
    mod.rs          # FederationClient struct + shared protocol types
    handshake.rs    # WebFinger resolution, token request/grant/store/issue
    shares.rs       # announce_share, send_share_accept, send_share_reject, send_revocation, announce_pictures, presign_remote_pictures, send_picture_edit_request
  resolver.rs       # self_register, update_mapping, verify_token

services/
  auth.rs / users.rs / pictures.rs / user_settings.rs / jobs.rs
  storage.rs        # feature 22: storage-quota enforcement math (committed+reserved), reservations,
                    #   warn levels, GET /me/storage payload
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
  routine.rs         # feature 17: generic Routine trait + RoutineHandle + per-key
                     # debounce/coalesce/rerun runtime — the one runtime all background work runs on
  routine/           # the concrete routines, grouped under the framework
    pipeline.rs      # Pipeline routine: per-user tag/announce reconcile; sweep = recovery/poll fallback
    exif_drain.rs    # feature 14: ExifDrain routine (deferred-EXIF-job drain)
    tag_rename.rs    # TagRename routine (trigger-only; run is todo!) + TagRenameInput
    unannounce.rs    # Unannounce routine (trigger-only; revocation-cascade tail) + UnannounceInput
    job_watchdog.rs  # JobWatchdogTask + JobCleanupTask routines (sweep-only)
    purge_sweep.rs   # PurgeSweepTask routine (sweep-only): physically purge owned, retention-expired
                     # trashed pictures — unannounce + delete tracking, S3 cleanup, hard-delete
    storage_reconcile.rs # feature 22: StorageReconcileTask (sweep-only) — recompute user_storage
                     # counters from scratch + refresh the Redis committed mirror
    pipeline/
      evaluation.rs  # per-user tag service evaluation + reconciliation, then announcement
      dedup.rs       # feature 11: content-dedup reconciler (serial per user) — survivor selection,
                     # rescue-promotion, arrival classification (boomerang guard)
      announcement.rs # inline reconcile_share: PFA/errored full pass + active dirty-delta (deliver-then-record)
```

## D) AppState

```rust
pub struct AppState {
    pub settings: Arc<Settings>,  // feature 23: layered runtime config (Config = Arc<Settings>)
    pub db: PgPool,
    pub redis: RedisClient,
    pub jwt: JwtService,
   pub worker_jwt: JwtService,
    pub storage: StorageClient,
    pub federation: FederationClient,
    pub resolver: ResolverClient,
   pub routines: Routines,   // feature 17: pipeline / exif_drain / tag_rename / unannounce trigger handles
}
```

**Runtime configuration (feature 23 §4).** `Config` is now `Arc<Settings>` — the layered
[`common::settings`](../common/src/settings.rs) engine (`default → env(locks) → DB override`,
`ArcSwap`-hot-swapped). **Core** secrets/topology stay env-only; **operational** fields (retention,
rate-limit + share caps, routine intervals/batches, `default_storage_quota_bytes`, trace peers, CORS
origins, `registration_mode`) are DB-editable from `/admin` and read live (`settings.get(keys::X)`).
Overrides live in `app_settings`; a `PATCH` rebuilds the snapshot. CORS is a dynamic middleware reading
the live origin list per request. Three primitives are shared with the resolver in `common`:
[`common::auth`](../common/src/auth.rs) (`JwtService`/claims/`TokenType`, incl. `ResolverDelegation` +
`ResolverAdminSession`), `common::routine`, and `common::settings`. **`AuthAdmin` is dual-issuer**: it
accepts a direct user-admin token **or** a backend-signed `ResolverDelegation` replayed by the resolver
proxy (`sub="resolver"`).

## E) Tagging pipeline

The pipeline is the `Pipeline` [`Routine`](#h-routine-framework-feature-17) (`infra/pipeline.rs`): `run(user_id)` evaluates enabled
tagging services against dirty pictures and reconciles tag assignments; `sweep` is its recovery/poll fallback.

**Dirty picture detection** — `pictures.last_pipeline_run_at IS NULL` on new/invalidated pictures; `tagging_services.last_invalidated_at` bumps on
config changes. Dirty = `last_pipeline_run_at IS NULL OR last_pipeline_run_at < last_invalidated_at` for any enabled service.

**Invalidation is intrinsic to the write, not the caller.** Every repository write that changes a
tagging-relevant input re-NULLs `last_pipeline_run_at` in the same statement: manual-tag mutations
(`TagRepository::batch_assign`/`batch_remove`/`promote_service_tags_to_manual`) and the EXIF/metadata
writes (`update_from_worker` extraction, `write_exif_snapshot` / batch EXIF write-through,
`set_filename`). Service callers only need to *trigger* the wake (the sweep is the backstop if a
trigger is dropped). This closes the gaps where a path mutated tags/EXIF but forgot to invalidate —
notably WebDAV write-back (`vfs` add/remove ops route through `batch_assign`/`batch_remove`) and worker
EXIF extraction landing after the first pipeline pass.

**Wake model** — `Input = Key = Uuid` (the user). Triggered via `routines.pipeline` (a `RoutineHandle<Uuid>`) with bounded concurrency
(`PIPELINE_CONCURRENCY`, default 4), serial per user, plus the `sweep` poll fallback (`PIPELINE_POLL_INTERVAL_SECS`, default 1 hour, + startup).
Triggered after: ingest, manual tag edit, service config change, inbound share announcement, `cleanup_incoming_share`. Interactive triggers (
`trigger`)
start a run promptly; worker-driven ones that arrive one-per-picture (EXIF/visual reconcile completion, thumbnail completion) use `trigger_debounced`,
coalescing a burst into a single run over a `PIPELINE_DEBOUNCE_MS` (default 5000) window. The window starts on the first debounced trigger and is
**not** reset (latency bounded to the window); an interactive trigger arriving mid-window promotes the run to start immediately.
`PIPELINE_DEBOUNCE_MS=0`
disables debouncing (used by tests). The per-key debounce/coalesce/rerun mechanics live in the generic runtime, not here.

**Re-announce on worker completion** — a `gen_thumbnail` completion usually first computes `file_hash`/`blurhash`/`thumbnails_generated_at`, which may
post-date a picture's first announce. If the picture is in the `share_announcements` tracking table, completion re-marks it dirty (debounced wake) so
the announcement delta re-delivers the refreshed metadata. The race-free backstop: the recovery sweep also re-dirties any tracking row whose
`announced_updated_at` trails the picture's `updated_at`.

**Evaluation order** — `SharedTagMapping` always first. Rule and Segmentation services in user-defined `position` order. Gating accumulates tags from
`manual` + `incoming_share` + earlier services; pipeline tags re-derived from scratch each run.

**Rule predicates** — a structured JSONB predicate tree (feature 13): logical `and`/`or`/`not`
composition over spatial nodes (`gps_bbox`, `gps_radius`) and typed field-condition leaves covering
all EXIF/file/ownership attributes. Parsed into `domain::predicate::Predicate` (validated on
create/update, evaluated against the `PipelineInput` projection). See
`doc/features/13_better_rules.md`.

**Service config (feature 20)** — every service type's payload lives in one `tagging_services.config`
JSONB column (the per-type child tables are dropped). `domain::tagging::ServiceConfig` is the single
hub: `parse` validates + normalizes raw JSON (rule predicates, assigned tags, segmentation bands),
`to_value` is storage-ready, and one `evaluate(input, incoming_share_ids)` dispatch covers all three
types (gating is `TaggingService::should_run`). The API edits config **uniformly** — create takes a
`config`, `PUT /tagging-services/{id}/config` replaces it; there are no per-rule/segment/mapping
sub-resources. `shared_tag_mapping` is **one service per incoming share** (scalar `incoming_share_id`

+ `assign_tags`), brokenness derived from the share's status. **Segmentation** is the calendar
  partition operator (resolves to 0 or 1 tag, first-covering-band-wins). See
  `doc/features/20_calendar_segmentation.md`.

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
deferred-job drain (`infra::exif_drain`, the `ExifDrain` `Routine` — `()`-keyed, triggered + interval
sweep) creates the reconcile jobs and flips them to `pending`. Received pictures take the
set-based local-override merge (or a propose-to-owner edit in `suggest` mode). Convergence is tracked
through the `exif_sync` histogram, not per-picture job ids. See `doc/features/14_better_batch_editing.md`.

**Physical copy & content dedup (11)** — `POST /pictures/{id}/copy` copies a received (or owned)
picture's bytes into the caller's library as a new owned identity with root-resolved `copy_source_*`
provenance (same-/cross-instance byte paths), then enqueues `gen_thumbnail`. The worker computes a
metadata-stripped `content_hash` (stable across EXIF edits, changes on visual re-encode), forwarded in
`AnnouncedPicture` so recipients group across owners. The **dedup reconciler** runs **serial per user
in the pipeline** (`infra::pipeline::dedup`). Each `content_hash` group (or `file_hash` fallback) is
**Live** (no rejection → one live survivor, rest `content_dedupe`) or **Rejected** (≥1
`manual`/`boomerang` → exactly one `manual` trash representative, rest `boomerang`). The reconciler is
**stable**: a correct single-live group is never reshuffled, so whichever copy is live — including one
the user chose via `POST /pictures/{id}/copies/keep` — stays live; survivor selection (§5.1) only runs
to *collapse* a transient multi-live group or *promote* when none is live (rescue-on-purge). For a
**Rejected** group the representative is the **best/priority** copy (`best()` over the whole group —
prefer not-owner-deleted, then **owned/local**, then original, then lowest id), so deleting content
the user also holds a local copy of trashes the **owned** copy as the representative (correct
owned-deletion trash messaging, not a misleading "owner's copy untouched" while a local copy hides as a
boomerang). Hidden rows (`content_dedupe`/`boomerang`) are excluded from **all** list/trash views
(`push_filters` shows live + `manual` only), so the trash shows one recoverable entry per rejected
group, not a pile of duplicates; the `GET /pictures/{id}/copies` endpoint is the one read that surfaces
the whole group. Lifecycle triggers maintain the invariant: **manual delete** (`reject_content_group`)
rejects the whole group — the priority copy → `manual`, the rest → `boomerang`, applied at delete time
with the same `best()` so the reconcile never replaces it; **restore** flips the `boomerang` siblings
back to `content_dedupe` (rejection lifted, rescue re-enabled); when a `manual` representative
disappears a `boomerang` is promoted to the new representative; and a copy **arriving** into a Rejected
group is itself `boomerang`'d (`classify_arrival`) — closing the gap the owner-match loop prevention
(§6.6) can't, since a copy launders the owner identity.
The recovery sweep re-wakes users whose groups need a promotion/collapse. An admin
`POST /admin/pictures/regenerate-thumbnails` bulk-(re)enqueues `gen_thumbnail` (which recomputes
`content_hash`) for missing-thumbnail or all owned pictures. See
`doc/features/11_physical_copy_and_dedup.md`.

**Storage quotas (22)** — authoritative per-user usage lives in `user_storage` (four billed cells:
originals/versions × live/trashed), maintained by row triggers on `pictures`/`picture_versions` so it
stays correct across every code path that touches `file_size`/`deleted_at`/`remote_picture_id` (the
picture-delete case is a `BEFORE DELETE` trigger, reading the picture's versions before the FK cascade
removes them). Received pictures (`remote_picture_id IS NOT NULL`) are never billed. Redis holds the
fast path: a cached `committed` mirror + per-upload `reserved` sub-keys; enforcement math is
`committed + reserved + incoming ≤ quota` (`services::storage`), applied at upload presign/complete,
WebDAV `PUT` (net delta), and `copy_picture`. The `storage_reconcile` routine recomputes the counters
daily (drift safety net). `Storage::prefix_usage` (paginated `ListObjectsV2`) backs the admin S3
storage-audit. `NULL`/`0` quota = unlimited. See `doc/features/22_storage_quotas.md`.

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

## H) Routine framework (feature 17)

The generic core (the `Routine` trait, `RoutineHandle`, scheduler, `spawn`) was **lifted to
[`common::routine`](../common/src/routine.rs)** (feature 23 §8) behind a `routine` cargo feature so the
resolver reuses it; `back/src/infra/routine.rs` re-exports it and keeps the concrete backend routines.
Routines read their `interval()` from the live settings snapshot each tick, so an interval change from
the dashboard takes effect after the current wait (no re-spawn). When `USE_RESOLVER=true`, `main` also
spawns the **`ResolverHeartbeat`** routine (startup + `resolver_heartbeat_interval_secs`), which mints a
fresh backend-signed `ResolverDelegation` token, gathers fleet metrics, and pushes them to the resolver
(feature 23 §3.2).

All background work runs on one generic runtime, `infra/routine.rs`. A **`Routine`** is a named unit
of work triggerable three ways: recurrently (every `interval()`), at startup (`run_on_startup()`),
and manually (`RoutineHandle::trigger`/`trigger_debounced`). Each trigger carries an `Input`; a dedup
`Key` is *derived* from it (`Routine::key`). Equal keys never run concurrently — while a key is
running a new trigger sets a **rerun** flag (storing the latest input) and the runtime re-runs once at
the end; a per-routine `debounce()` window coalesces a burst before the first run. `concurrency()`
bounds distinct keys in flight (per-key is always serial). The periodic/startup `sweep` enumerates
the inputs needing a run (default: `trigger(Default)`, right for `()`-keyed routines; the pipeline
overrides it to enumerate dirty/dedup-needing users).

`AppState.routines: Routines` holds the trigger handles (`pipeline`, `exif_drain`, `tag_rename`,
`unannounce`); the sweep-only routines (job watchdog/cleanup, purge sweep) expose no handle.
`routine::spawn` spawns each runtime onto the Tokio runtime and returns its handle plus a
`JoinHandle`; `main` collects the handles into `Routines` and keeps the join handles. On SIGINT/SIGTERM
`main` drives `axum`'s `with_graceful_shutdown`, then flips the shared `shutdown` watch and awaits each
join handle — the runtime stops its loops and drains in-flight runs before exiting.

**Durability.** Triggers are in-memory (`mpsc`) — a crash drops queued triggers. A routine needing
crash-safety provides a `sweep` that re-derives its outstanding work from the DB (pipeline, exif
drain, job/purge sweeps do). `tag_rename`/`unannounce` are trigger-only and best-effort (exactly the
old `TaskQueue` behaviour); making tag-rename durable is a noted follow-up. See
`doc/features/17_unified_routine_framework.md`.

## I) Coding guidelines

Applies to `back/` and `worker/` (the whole Rust workspace, including `archypix-common`).

### Database migrations

Schema changes go into new migration files by default; only edit an already-applied migration if explicitly asked or if not yet applied in production.

```bash
cargo sqlx migrate add -r --sequential <name>   # creates xxx_<name>.up.sql / .down.sql
```

`back/migrations/schema.sql` is a generated, non-authoritative snapshot of the **full current schema**
— read this file (not the individual migration files) when you need to see the schema as it stands
today. Regenerate it after every migration:

```bash
docker exec -i archypix-postgres pg_dump -U archypix -d archypix_back --schema-only --no-owner \
  --no-privileges --no-comments --schema=public --exclude-table=_sqlx_migrations \
  | grep -vE '^--|^SET |^SELECT pg_catalog\.set_config|^\\restrict|^\\unrestrict' | cat -s \
  > back/migrations/schema.sql
```

After adding a migration:

1. **Apply it to the dev DB and regenerate the offline cache** (from `back/`, `DATABASE_URL=…/archypix_back`):
   `cargo sqlx migrate run && cargo sqlx prepare -- --tests`
   (`-- --tests` captures test query macros in `.sqlx`; verify with
   `env -u DATABASE_URL SQLX_OFFLINE=true cargo check --tests -p archypix-back`).
2. **Regenerate `schema.sql`** (command above) so it reflects the new schema.

### Rust guidelines

- Follow Rust best practices. Always favor refactoring over sticking to existing legacy functions.
- For modules with sub-files, use a `module_name.rs` file alongside the `module_name/` directory instead of placing a `mod.rs` inside the directory.
- Keep repository separated from services: don't create too specific repository functions, instead create general ones that can be reused. Don't
  reference services in a repository function: if a function is made for a specific task today, it may be used elsewhere tomorrow, so make them
  factorized and general rather than specific to a given service.

### Tracing

Use `#[tracing::instrument]` with `fields(...)` for identifying context instead of logging it at
the call site: don't repeat a field already on the span (own or ancestor's); log calls should
just carry genuinely new info (errors, counts, computed values). Use empty fields +
`Span::current().record(...)` for values only known partway through the function.

In `fields(...)`, a bare `name` declares an empty field — it does **not** capture the in-scope
variable (unlike `span!`/`event!`). Use `name = value` (or `%name`/`?name` shorthand) to actually
record it.

`AppError`-based error responses are already logged by `AppError::into_response()` — no need for
an extra `warn!` next to a function that just returns `AppError`.

Federation calls propagate trace context via headers (`trace_headers_for`/
`maybe_set_remote_parent` in `back/src/infra/observability.rs`, gated on the JWT-verified peer);
worker jobs propagate it through the DB job row instead.

### Common mistakes

- Global domain comparaison can't tell if the instances are the same. bob_global_domain == alice_global_domain does not tell if bob and alice are on
  the same instance. Multiple instances can have the same global domain. Use `services::users::find_local_user_id` instead to check if a user is on
  the same instance.

### Environment

For things involving the `archypix-worker` crate, run in `nix develop`.

### Agents — working on back/worker

- Keep code comments short and sparse — see the shared rule in doc/00_CODING_GUIDELINES.md.
- When editing the API, update doc/06_API_REFERENCE.md.
- Keep tests up to date: new features and modified behaviour should be reflected in the test suite.
- Keep documentation up to date, matching the level of detail already present.
- When completing a task, update doc/99_ROADMAP_MVP.md, and add things not yet implemented to it.
