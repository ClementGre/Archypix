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
- [ ] **Better Rules** — Add more pipeline rule tagging rules. Allow to do ands and ors of rules (curently only ORs ?). Better UI for
  rules. Fix UI for segmentations.
- [ ] **Tag rename cascade** — API endpoint for `TaskQueue::TagRename`; cascade to shares, segments, hierarchies.
- [ ] **Federation robustness** — do not fail list pictures with 500 when the inbound picture remote presign fails, token refresh schedule, retry
  logic, presigned URL caching for remote pictures.
- [ ] **Multi-picture edits** — The frontend offers very limited multi-picture viewing and editing support. The idea would be to see all common tags
  and metadata of selected pictures. See tags not on all pictures also, and show mixed exif data as mixed (maybe with a popup showing the different
  values if there is not too much). The idea would be to take the current right tab for a single picture, and use about the same interface for when
  selecting multiple pictures. Endpoints should be added to the API to support batch tag read, batch exif read/write, and other info batch read (file
  size, ...).
- [ ] **Logging robustness** — Better tracing, logs that does not mix up with multi-threading (Otel compatibility?).

## To-do for v1.0

- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots in MinIO.
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in `edit_picture` worker.
- [ ] **Physical copy & content dedup** — "rescue" copy of a received picture into your own library
  (new distinct identity); `content_hash`-based dedup of identical copies (one live survivor,
  reversible `content_dedupe` hiding, rescue-on-purge), and the deleted-content `boomerang` guard.
  Schema already in 001 (via the trash migration). See `doc/features/11_physical_copy_and_dedup.md`.
- [ ] **Storage quotas** — per-user storage quotas, webdav quota properties in PROPFIND. Allow resolver to update quotas (for smart-resolver
  features).
- [ ] **Registration rules** – open registration vs invite-only (requires an invite code/link).
- [ ] **Versioning better support** — presign and CRUD on versions. Frontend viewing and editing versions.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDav** — support for Directory-level operations (DELETE/MOVE/COPY on collections), Conditional & range requests (`If-Match`/
  `If-None-Match`, `Overwrite`, `Range`), and Real LOCK/UNLOCK enforcement
- [ ] **Rate limiting & validators** — More rate limiting
