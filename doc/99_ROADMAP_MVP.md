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
- [x] **Hash & size reliability** — authoritative `file_size` read from S3 (`HEAD`) on upload instead of trusting the client; client-computed SHA-256
  sent on upload as a provisional `file_hash` (re-confirmed by the worker, byte-identical digest); `gen_thumbnail` reports size/hash even for
  non-thumbnailable formats (skips thumbnails instead of failing); federation announcements carry `file_hash` + `thumbnails_generated_at` so received
  pictures get an ETag and known thumbnail availability; WebDAV overwrite is a no-op on identical hash (no spurious version/re-upload);
  `gen_thumbnail`
  completion re-announces tracked pictures (with a recovery-sweep backstop); pipeline wakes are debounced (`PIPELINE_DEBOUNCE_MS`) for worker-driven
  bursts while interactive wakes stay prompt.
- [x] **Trash & restore** — soft delete with `deleted_at`/`deleted_reason`; owner-deletion propagation
  (announced `owner_deleted_at`/`owner_purge_at` grace window + warning badge, kept in share coverage
  until the `purge_sweep` recurring task physically deletes owned pictures past their retention); and
  recipient EXIF overrides (owner-authoritative `remote_exif_data` snapshot + sticky per-field
  `local_exif_overrides`, materialised into `exif_data`; DB-only, no `edit_picture` job). Trash/restore
  + local-override endpoints, `trash_retention_days` setting. Ships the single consolidated schema
    migration that also covers the two items below. **Frontend:** dedicated Trash page, trash/restore
    actions (selection panel, lightbox, batch), owner-deletion warning badge (red owner chip + grace
    banner), recipient EXIF overrides editable inline with an "overwritten" tag (+ WebDAV-caveat
    tooltip), retention setting. See `doc/features/09_trash_and_exif_overrides.md`.
- [x] **Recipient EXIF editing** (backend) — per-share `allow_exif_edit` grant (propagated to the
  incoming share); `POST /pictures/{id}/exif` with `mode: local | propose`; `pictures/edit_request`
  federation verb with same-backend short-circuit; owner re-verifies the grant and applies via
  `edit_picture` write-through (re-announce to all); escalate clears the local override. Frontend
  toggle + received-picture editor pending. See `doc/features/10_recipient_exif_editing.md`.
- [x] **Logging robustness** — Better tracing, logs that does not mix up with multi-threading, Otel compatibility.
  - [x] Span-tracing & Otel compatibility.
  - [x] Webdav/VFS proper span tracing.
  - [x] Federation/Clients proper span tracing and linkage between instances.
  - [x] Bounded operation-name cardinality (matched route, not raw path), HTTP semconv attributes,
    response/error status, client span kinds, richer resource attributes.
  - [x] Doc and best practices update.
  - [ ] Trace sampling + OpenTelemetry Collector (tail sampling, SpanMetrics RED metrics) —
    deployment hardening, not yet implemented. See `doc/features/16_trace_sampling_and_collector.md`.
- [x] **Better Rules** — Replaced text predicates with a structured JSONB predicate tree supporting
  arbitrary AND/OR/NOT composition; extended field coverage to all EXIF/file/ownership attributes
  (camera brand/model, ISO, f-number, focal length, exposure time, mime type, file size, dimensions,
  `is_owned`, GPS radius); added `is_present` check and the `gps_radius` spatial predicate.
  Schema migration `0002` (`rule_tagging_services.predicate TEXT → JSONB`, legacy forms converted);
  `PipelineInput` extended with the new fields; validation on create/update (type compatibility,
  ranges, regex, depth ≤ 10). **Frontend:** a nested block composer (`PredicateBuilder`) for AND/OR/NOT
  groups, field-condition + GPS-area leaves, drag-to-reorder within a group; rules render as readable
  expressions. Also fixed the segmentation editor crash (empty-string `<Select.Item>` value).
  See `doc/features/13_better_rules.md`.
- [x] **Multi-picture edits** (feature 14) — _Backend_: the `PictureSelection`/`PictureFilter` model
  (`services::selection`), `POST /pictures/aggregate` (summary/tags/exif sections, ancestor-expanded tag counts with
  `manual_count`, type-aware field stats), selection-based batch writes (tristate tags, EXIF with `local`/`suggest`
  modes, batch trash/restore) with `dry_run`, and the deferred-EXIF-job model
  (`exif_sync_status = 'pending_job_creation'` + `infra::exif_drain`). _Frontend_: the `selection` store now holds the
  descriptor (query + include/exclude deltas); `⌘/Ctrl+A` / a desktop+mobile **floating action bar** (count, Select-all,
  Invert, Clear, Batch actions) adopt the view's `PictureFilter` instead of enumerating ids; the **multi-select right
  panel** (`components/photos/batch/`) reuses the single-picture section layout fed by `useAggregate` (lazy per-section
  Summary / tristate Tags / type-aware EXIF with the GPS bbox on `MapView`); a `BatchExifDialog` edits EXIF across the
  selection (local/suggest mode); and every batch write is gated by a **mandatory confirmation popup** hosting the
  endpoint's `dry_run` preview (`BatchConfirmDialog`). See `doc/features/14_better_batch_editing.md`.
- [x] **Dedup pictures at upload time** — the batch presign (`POST /uploads/batch`) takes each file's SHA-256 and `initial_tags`; a hash already on an
  owned picture comes back `duplicate: true` (no S3 slot) with the initial tags landed on the existing picture, a trashed match restored, and
  identical
  files within one batch collapsed to a single slot. The upload dialog computes the hash up front and shows an amber check for deduplicated files.
- [ ] **Tag rename cascade** — API endpoint for `TaskQueue::TagRename`; cascade to shares, segments, hierarchies.
- [ ] **Federation robustness** — do not fail list pictures with 500 when the inbound picture remote presign fails, token refresh schedule, retry
  logic, presigned URL caching for remote pictures.

## To-do for v1.0

- [x] **Physical copy & content dedup** — "rescue" copy of a received picture into your own library
  (`POST /pictures/{id}/copy`, new distinct owned identity with root-resolved `copy_source_*`
  provenance; same-/cross-instance byte paths). Worker computes a metadata-stripped `content_hash`
  (`AnnouncedPicture` carries it); a serial-per-user pipeline reconciler (`infra::pipeline::dedup`)
  keeps one live survivor per group and hides the rest as `content_dedupe`, with rescue-on-purge
  promotion. The deleted-content `boomerang` guard at the announce-receive path, plus the
  user-clarified manual-delete→boomerang of `content_dedupe` siblings (sticky rejection that outlives
  the manual twin). Schema already in 001 (via the trash migration). **Frontend:** copy/"rescue"
  action in the selection panel + lightbox, owner-deleting grace-banner rescue button, copy-of
  provenance line. See `doc/features/11_physical_copy_and_dedup.md`.
- [x] **Unified routine framework** — one generic `Routine` runtime (`infra/routine.rs`) with
  recurrent/startup/manual triggers and per-key debounce/coalesce/rerun, replacing the four
  hand-rolled mechanisms (pipeline waker + per-user scheduler, recurring `Scheduler`, in-process
  `TaskQueue`, exif-drain waker). Routines: pipeline, exif drain, job watchdog/cleanup, purge sweep,
  tag rename, unannounce. See `doc/features/17_unified_routine_framework.md`.
- [ ] **Storage quotas** — per-user storage quotas, webdav quota properties in PROPFIND. Allow resolver to update quotas (for smart-resolver
- [ ] **Registration rules** – open registration vs invite-only (requires an invite code/link).
- [ ] **Versioning better support** — presign and CRUD on versions. Frontend viewing and editing versions.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDav** — support for Directory-level operations (DELETE/MOVE/COPY on collections), Conditional & range requests (`If-Match`/
- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots in MinIO.
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in `edit_picture` worker.
  features).
  `If-None-Match`, `Overwrite`, `Range`), and Real LOCK/UNLOCK enforcement
- [ ] **Rate limiting & validators** — More rate limiting
- [~] **Video & audio playback** — **Tier 1 (done):** inline progressive playback of the original from
  S3 (HTTP-Range, no transcode infra) via `@vidstack/react` in the Lightbox (autoplay) and details
  panel (audio inline; video → poster opening the Lightbox). Only browser-playable codecs work.
  **Tier 2 (todo):** a `transcode` worker job (ffmpeg) producing a web-friendly MP4 derivative +
  poster-frame thumbnail for non-decodable uploads (`.mov`/HEVC, `.avi`, `.mkv`). **Tier 3 (later):**
  HLS adaptive streaming (Vidstack already supports it). See `doc/05_FRONTEND_ARCHITECTURE.md §9`.
