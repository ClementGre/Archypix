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
  `doc/features/01_better_sharing_support.md`.
- [x] **EXIF editing** — write-through single + batch edit with convergence guarantees and MIME preflight. See
  `doc/features/04_better_exif_support.md`.
- [x] **Admin endpoints** — user management, job status, instance metrics.
- [x] **Hierarchies** — mirror/query/static node-tree config, read resolver, CRUD + `tree`/`browse` endpoints, write-back schema. See
  `doc/features/05_hierarchies.md`.
- [x] **WebDAV** — per-hierarchy endpoint, proxy reads, tag write-back, hash-dedupe, versioning on overwrite. See `doc/features/06_webdav.md`.
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
  editing..
- [ ] **Tag rename cascade** — API endpoint for `TaskQueue::TagRename`; cascade to shares, segments, hierarchies.
- [ ] **Federation robustness** — don't 500 on failed remote presign, token refresh schedule, retry logic, presigned URL caching for remote pictures.

## To-do for v1.0

- [x] **Physical copy & content dedup** — rescue-copy into own library with `copy_source_*` provenance, `content_hash`-based dedup reconciler,
  boomerang guard. See `doc/features/11_physical_copy_and_dedup.md`.
- [ ] **Storage quotas** — per-user storage quotas, WebDAV quota properties in PROPFIND, resolver-updatable quotas.
- [ ] **Registration rules** — open registration vs invite-only (invite code/link).
- [ ] **Versioning better support** — presign and CRUD on versions; frontend viewing and editing.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDav** — directory-level DELETE/MOVE/COPY, conditional/range requests, real LOCK/UNLOCK.
- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots in MinIO.
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in `edit_picture` worker.
- [ ] **Rate limiting & validators** — more rate limiting.
- [~] **Video & audio playback** — **Tier 1 done:** inline progressive playback via `@vidstack/react`. **Tier 2 todo:** ffmpeg transcode worker for
  non-decodable formats. **Tier 3 later:** HLS adaptive streaming. See `doc/05_FRONTEND_ARCHITECTURE.md §9`.
