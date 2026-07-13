# Backend + Resolver MVP Roadmap

## Completed

- [x] Core infrastructure: layered Rust architecture, Axum router, SQLx migrations, AppState (Postgres, Redis, MinIO, JWT, federation, resolver
  clients).
- [x] Auth, users, pictures, tags, shares, settings, admin endpoints; federation auth handshake; resolver user-management endpoints.
- [x] Picture upload pipeline: presigned staging → server-side copy → optional versioning.
- [x] Resolver self-registration and tests.
- [x] Worker foundation: Postgres-backed job queue, HTTP-only worker crate.
- [x] Tag sharing: accept flow, picture announcement, same-backend short-circuit, `/SharedToMe/…` tags, cross-instance presigning.
- [x] Tests: domain unit tests, repository/service integration tests, worker HTTP contract tests, federation end-to-end and security tests.

## To-do for the MVP

- [x] **Tagging pipeline CRUD** — API to create/manage tagging services (rules and segmentation).
- [x] **Tagging pipeline execution** — event-driven pipeline evaluator on ingest/edit/share events.
- [x] **Tagging pipeline tag removal** — per-source tag lifecycle (stale removal, disable/delete semantics).
- [x] **Better sharing** — per-picture token presign, pipeline-driven announce/unannounce, ShareBack, loop prevention, transitive sharing. See
  `doc/features/01_better_sharing_support.md`. `future=false` shares still re-announce metadata/deletion changes to already-tracked
  pictures (only *new additions* are withheld); tombstone (rejection) deletes the outgoing tracking rows like revoke.
- [x] **EXIF editing** — write-through single + batch edit with convergence guarantees and MIME preflight. See
  `doc/features/04_better_exif_support.md`.
- [x] **Admin endpoints** — user management, job status, instance metrics.
- [x] **Hierarchies** — mirror/query/static node-tree config, read resolver, CRUD + `tree`/`browse` endpoints, write-back schema. See
  `doc/features/05_hierarchies.md`.
- [x] **WebDAV** — per-hierarchy endpoint, proxy reads, tag write-back, hash-dedupe, versioning, atomic-save staging. See `doc/features/06_webdav.md`,
  `doc/features/08_webdav_issues.md`.
- [x] **Hierarchy improvements** — `drop` inbox nodes, per-node write-back tri-state, writable `matchUntagged`, mirror `maxDepth`/foreign excludes.
  See `doc/features/18_hierarchy_improvements.md`.
- [x] **Better workers** — multi-backend support, global semaphore, burst-friendly polling.
- [x] **Security audit** — security, privacy, reliability (rate limiting and throttling) fixes.
- [x] **Shares name and message** — required `name` and optional `message` on shares, propagated to the recipient.
- [x] **Incoming-share detail enrichment** — `future`, `shared_tag_path`, `last_announcement_received_at`, `shareback_of` surfaced on `IncomingShare`;
  richer frontend share popovers.
- [x] **Hash & size reliability** — authoritative S3-read `file_size`, worker-confirmed `file_hash`, thumbnail-skip-not-fail, debounced pipeline
  wakes.
- [x] **Trash & restore** — soft delete, owner-deletion propagation with grace window, recipient EXIF overrides, trash retention setting. See
  `doc/features/09_trash_and_exif_overrides.md`.
- [x] **Recipient EXIF editing** (backend) — per-share `allow_exif_edit` grant, `local`/`propose` edit modes, owner re-verification and re-announce.
  See `doc/features/10_recipient_exif_editing.md`.
- [x] **Logging robustness** — span tracing, Otel compatibility, WebDAV/federation span linkage, bounded operation-name cardinality.
  - [ ] Trace sampling + OpenTelemetry Collector — deployment hardening, not yet implemented. See `doc/features/16_trace_sampling_and_collector.md`.
- [x] **Better Rules** — structured JSONB predicate tree (AND/OR/NOT), full EXIF/file/ownership field coverage, GPS radius, frontend
  `PredicateBuilder`. See `doc/features/13_better_rules.md`.
- [x] **Multi-picture edits** (feature 14) — `PictureSelection`/`PictureFilter` model, batch aggregate/tags/EXIF/trash with dry-run, deferred-EXIF-job
  drain, frontend floating action bar + batch panel. See `doc/features/14_better_batch_editing.md`.
- [x] **Dedup pictures at upload time** — batch presign hashes files up front, dedupes within-batch and against existing/trashed pictures.
- [x] **Calendar segmentation & unified service config** — Calendar partition operator, unified `tagging_services.config` JSONB, uniform config
  editing.
- [x] **Tag rename cascade** — `POST /tags/rename` search-and-replaces a tag subtree across manual tags, shares, pipeline configs, hierarchy configs;
  invalidates + wakes pipeline. Frontend rename dialog in tag tree menu.
- [ ] **Federation robustness** — don't 500 on failed remote presign, token refresh schedule, retry logic, presigned URL caching for remote pictures.
  Clarify whether to use the federation_messages table, or to delete it.

## To-do for v1.0

- [x] **Physical copy & content dedup** — rescue-copy into own library with provenance, content-hash dedup reconciler, boomerang guard. See
  `doc/features/11_physical_copy_and_dedup.md`.
- [x] **Storage quotas** — trigger-maintained per-user counters, Redis fast path, enforcement at upload/WebDAV/copy, daily reconcile, admin quota +
  audit UI. See `doc/features/22_storage_quotas.md`.
- [x] **Resolver's admin dashboard** — backend+resolver+frontend done: layered resolver rebuild, delegation-token auth, operator sessions,
  `/api/resolver-admin/*` (overview/backends/settings/invites/config-matrix), placement strategies. See
  `doc/features/23_resolver_admin_and_runtime_config.md`, `doc/features/24_resolver_admin_frontend.md`.
- [x] **Registration rules** — open/invite/admin-invite modes, invite codes, instance-pinned invites, invite graph; backend+resolver+frontend. Spec
  §6–7.
- [x] **Admin config instead of envs** — unified `common::settings` engine (typed keys, env-locks, DB override, hot-swap) across
  backend/worker/resolver + metadata-driven `SettingsPanel`. Spec §4.
- [x] **Resolver chore** — resolver's entire router nested under one `/archypix-resolver/` prefix (no more
  `.well-known/webfinger`); fixed `GET /archypix-resolver/info` (bootstrap discovery, backend + resolver) and
  `GET /archypix-resolver/resolve` (federation/login hot path). Frontend bootstraps `/info` for login/register/resolution,
  gates the Fleet dashboard button on `is_resolver`, keeps the register instance editable when registration is closed,
  replaces the static CORS caveat with a live reachability/CORS ping, and lets the fleet dashboard target any resolver
  domain. See `doc/features/25_resolver_chore.md`.
- [x] **Picture creator** — owner-vs-creator attribution field: default-to-owner, format convention
  (`@user:domain` / `#name` / plain), propagated on announcements + locally overrideable by recipients
  (propose-to-owner deferred, phase 2); consumed by public-share uploads. Prerequisite for public shares.
  Backend (`pictures.creator`/`creator_override`, `AnnouncedPicture.creator`, `POST
  /pictures/{id}/creator`, resolved creator on detail+list, copy-carries-creator, tests) + frontend
  info-panel `CreatorField` (linkify / edit / reset, client-side sigil guard). See
  `doc/features/26_picture_creator.md`.
- [ ] **Public shares** — link-gated *pull* shares served by the owner backend (live coverage, no
  `IncomingShare`): unauthenticated view + anonymous contribution (`creator = #name`, dedup-rejected),
  authenticated convert (save a copy / subscribe to a **derived** share / subscribe + share-back). One
  `allow_originals` tier vs a view-only gallery; recipient-initiated `shares/public/claim` verb. Depends
  on feature 26. See `doc/features/27_public_shares.md`.
- [ ] **Photos fix tools** — quick tools for fixing missing EXIF info in files (feature 21).
- [ ] **Versioning better support** — presign and CRUD on versions; frontend viewing and editing.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDav** — directory-level DELETE/MOVE/COPY, conditional/range requests, real LOCK/UNLOCK.
- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots in MinIO.
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in `edit_picture` worker.
- [ ] **Rate limiting & validators** — more rate limiting.
- [~] **Video & audio playback** — Tier 1 done (inline playback via `@vidstack/react`); Tier 2 todo (ffmpeg transcode worker); Tier 3 later (HLS). See
  `doc/05_FRONTEND_ARCHITECTURE.md §9`.

## Toward a real product (adoption, mobile, hosting)

- [ ] **Onboarding & opinionated defaults** — fresh instance auto-organizes out of the box (default hierarchy, date segmentation, starter rules) with
  progressive disclosure; user-facing terminology pass (rename tagging service / hierarchy / incoming share / predicate to plain words).
- [ ] **Bulk library import** — Google Takeout + local-folder import so a new user can land an existing library.
- [ ] **External shared-album import** — one-time, user-initiated import of Google Photos (Picker API) and public iCloud Shared Albums as received
  shares (external platform modelled as a pseudo-instance → synthetic `IncomingShare`); offered at onboarding to seed the account. Perpetual
  auto-bridge
  is later / demand-driven (ToS-fragile).
- [ ] **PWA viewer** — mobile view / receive / organize as a PWA; makes iOS (and any) users full participants without a native app.
- [ ] **iOS background uploader** — first native app (Android is covered by WebDAV + a folder-album gallery like Samsung/Fossify); reliable
  camera-roll
  backup into Archypix.
- [ ] **Backup & recovery discipline** — prerequisite before hosting paying users: S3 object versioning + replication to a store the backend has no
  delete rights on, Postgres point-in-time recovery, purge-sweep guardrails (dry-run, rate caps, alerts).
- [ ] **Managed hosting (archypix.com)** — fully-managed instance for non-technical cluster members; pricing = small monthly base + per-GB above a
  free
  floor + a monthly ceiling. Needs a legal entity (micro-entreprise on-ramp). Optional niche tier: bring-your-own S3 bucket per user.
