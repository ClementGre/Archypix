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
- [x] **Tagging pipeline tag removal** — per-source tag lifecycle: stale tags removed atomically on re-run; disable drops tags; delete promotes or
  removes.
- [x] **Better sharing** — per-picture token presign model, pipeline-driven announce/unannounce, ShareBack auto-accept, loop prevention, transitive
  sharing/revocation. See `doc/features/01_better_sharing_support.md`.
- [x] **EXIF editing** — write-through single + batch edit with `set`/`clear`; worker reconciles S3 original; convergence/revert guarantees; MIME
  preflight. See `doc/features/04_better_exif_support.md`.
- [x] **Admin endpoints** — user management, job status, instance metrics.
- [ ] **Full frontend** — v1 MVP frontend.
- [x] **Hierarchies** — mirror/query/static node-tree config, read resolver, CRUD + `tree`/`browse` endpoints, write-back schema. See
  `doc/features/05_hierarchies.md`.
- [x] **WebDAV** — per-hierarchy webdav endpoint, proxy reads, tag write-back, hash-dedupe, OS-junk filtering, versioning on overwrite, mirror subdir
  auto-tag on new path, case-insensitive write-side tag reuse. See `doc/features/06_webdav.md`.
- [x] **Better workers** — multi-backend support (comma-separated), global semaphore, burst-friendly polling.
- [x] **Security audit** — audit and fix: security, privacy, reliability (rate limiting and throttling).
- [x] **Shares name and message** — required `name` and optional `message` on shares, propagated from the
  `OutgoingShare` to the recipient's `IncomingShare` (same-backend and over federation), so users know what
  they're sharing/receiving and why. Set at creation.
- [x] **Incoming-share detail enrichment** — the recipient's `IncomingShare` now also stores `future`, the
  advisory `shared_tag_path` (the local `/SharedToMe/…` tag, set at creation and refreshed on each
  announcement so a sender-side rename is reflected), `last_announcement_received_at`, and `shareback_of`
  provenance; `shareback_of` is persisted on the `OutgoingShare` too. Frontend: richer share popovers
  (status badge, ShareBack/future flags, shared tag, last-received, ShareBack provenance), inline status
  badge removed from cards, a **Share back** action in the incoming popover, and a **Share back of** combobox
  in the create-share dialog.
- [ ] **Trash & restore** — soft delete with `deleted_at`, recipient notification, physical copy option.
- [ ] **Tag rename cascade** — API endpoint for `TaskQueue::TagRename`; cascade to shares, segments, hierarchies.
- [ ] **Federation robustness** — token refresh schedule, retry logic, presigned URL caching for remote pictures.

## To-do for v1.0

- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots in MinIO.
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in `edit_picture` worker.
- [ ] **Storage quotas** — per-user storage quotas, webdav quota properties in PROPFIND. Allow resolver to update quotas (for smart-resolver
  features).
- [ ] **Registration rules** – open registration vs invite-only (requires an invite code/link).
- [ ] **Versioning better support** — presign and CRUD on versions. Frontend viewing and editing versions.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDav** — support for Directory-level operations (DELETE/MOVE/COPY on collections), Conditional & range requests (`If-Match`/
  `If-None-Match`, `Overwrite`, `Range`), and Real LOCK/UNLOCK enforcement
- [ ] **Rate limiting & validators** — More rate limiting
