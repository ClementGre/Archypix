# Roadmap

One line per item. Details live in the matching `doc/features/NN_*.md`.

## Next

- [ ] **Timeline view** — structured browse over any tag subtree (a View dropdown, plus Group by
  merged into the sort menu) + the gallery filter-param rewrite. Depends on tag metadata. See
  `features/35_timeline_view.md`.
- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots.

## Toward a real product (adoption, mobile, hosting)

- [ ] **Onboarding & opinionated defaults** — fresh instance auto-organizes out of the box, with
  progressive disclosure and a user-facing terminology pass.
- [ ] **Bulk library import** — Google Takeout + local-folder import.
- [ ] **External shared-album import** — one-time import of Google Photos and public iCloud Shared
  Albums as synthetic received shares; perpetual auto-bridge is demand-driven (ToS-fragile).
- [ ] **PWA viewer** — mobile view / receive / organize without a native app.
- [ ] **iOS background uploader** — reliable camera-roll backup (Android is covered by WebDAV).
- [ ] **Backup & recovery discipline** — S3 versioning + replication the backend cannot delete from,
  Postgres PITR, purge-sweep guardrails. Prerequisite before hosting paying users.
- [ ] **Managed hosting (archypix.com)** — managed instances; base + per-GB pricing with a ceiling.
  Needs a legal entity.

## Later

- [ ] **Versioning better support** — presign and CRUD on versions; frontend viewing and editing.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDAV** — directory-level DELETE/MOVE/COPY, conditional/range requests, real
  LOCK/UNLOCK. Collection `MOVE` now `405`s (it used to rename a transient pending-dir marker), so a
  Finder folder rename keeps the name `MKCOL` minted — wiring it to `webdav_dir_name` is the fix.
  - [ ] **Directory CTag** — derived per-collection change token; must key off `updated_at`, not
    `file_modified_at` (feature 32).
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in the `edit_picture` worker.
- [ ] **Rate limiting & validators** — structured framework, limiters listed in the admin dashboard
  with window size + limit.
- [ ] **Trace sampling + OpenTelemetry Collector** — deployment hardening. See
  `features/16_trace_sampling_and_collector.md`.
- [~] **Video & audio playback** — Tier 1 done (inline `@vidstack/react`); Tier 2 ffmpeg transcode
  worker todo; Tier 3 HLS later. See `05_FRONTEND_ARCHITECTURE.md §9`.


## Known issues

- [ ] A reconcile already claimed when an external (WebDAV) overwrite lands still writes its older
  target onto the new file (31 §5, narrowed by 33 §6.4).
- [ ] A batch EXIF dry run previews a video under `edited` and counts it under `synced` — both read the
  stored status, but a video only earns its verdict once a write is attempted (33 §10).
- [ ] A physical copy inherits a `pending` source status without an accompanying reconcile job, so it
  can sit in `pending` until the user resyncs (33 §4.1).

## Done

- [x] **Core infrastructure** — layered Rust architecture, Axum router, SQLx migrations, AppState.
- [x] **Auth, users, pictures, tags, shares, settings, admin** — plus the federation auth handshake and
  resolver user-management endpoints.
- [x] **Upload pipeline** — presigned staging → server-side copy → optional versioning.
- [x] **Resolver self-registration**.
- [x] **Worker foundation** — Postgres-backed job queue, HTTP-only worker crate.
- [x] **Better workers** — multi-backend support, global semaphore, burst-friendly polling.
- [x] **Tests** — domain unit, repository/service integration, worker HTTP contract, federation
  end-to-end and security.
- [x] **Tagging pipeline** — CRUD, event-driven execution, per-source tag lifecycle.
- [x] **Better sharing** — per-picture token presign, announce/unannounce, ShareBack, transitive
  sharing. See `features/01`.
- [x] **Shares name and message** · **incoming-share detail enrichment** — propagated to the recipient;
  richer frontend share popovers.
- [x] **EXIF editing** — write-through single + batch with convergence guarantees. See `features/04`.
- [x] **Robust EXIF sync** — `file_exif` snapshot, claim-time target, `write_failed` + manual
  Retry/Revert-to-file. See `features/31`.
- [x] **EXIF read path & engine fallback** — ExifTool dispatch + fallback, observed read states, one
  `JobOutcome`/`JobProduct` response endpoint. See `features/33`.
- [x] **Hierarchies** — node-tree config, read resolver, CRUD + `tree`/`browse`. See `features/05`.
- [x] **Hierarchy improvements** — `drop` nodes, per-node write-back tri-state, mirror depth/excludes.
  See `features/18`.
- [x] **WebDAV** — per-hierarchy endpoint, proxy reads, tag write-back, hash-dedupe, atomic-save
  staging. See `features/06`, `features/08`.
- [x] **WebDAV last-modified** — `getlastmodified` follows the bytes, not a re-tag. See `features/32`.
- [x] **Trash & restore** — soft delete, owner-deletion propagation, retention setting. See
  `features/09`.
- [x] **Better trash** — trash as a filter over the main view (`?trash=exclude|include|only`), no
  separate page.
- [x] **Recipient EXIF editing** — per-share `allow_exif_edit`, `local`/`propose` modes. See
  `features/10`.
- [x] **Admin endpoints** — user management, job status, instance metrics.
- [x] **Security audit** — security, privacy, reliability fixes. See `features/07`.
- [x] **Logging robustness** — span tracing, OTel compatibility, bounded operation-name cardinality.
  See `features/12`.
- [x] **Hash & size reliability** — authoritative S3-read `file_size`, worker-confirmed `file_hash`,
  debounced pipeline wakes.
- [x] **Better rules** — structured JSONB predicate tree, full field coverage, frontend
  `PredicateBuilder`. See `features/13`.
- [x] **Multi-picture edits** — `PictureSelection`/`PictureFilter`, batch aggregate/tags/EXIF/trash with
  dry-run. See `features/14`.
- [x] **Dedup at upload time** — batch presign hashes up front, dedupes within-batch and against
  existing/trashed pictures.
- [x] **Calendar segmentation & unified service config** — Calendar partition operator, unified
  `tagging_services.config`. See `features/20`.
- [x] **Tag rename cascade** — `POST /tags/rename` across manual tags, shares, pipeline and hierarchy
  configs.
- [x] **Federation robustness** — degrading read paths, one typed versioned message envelope, outbound
  HTTP hardening, rate limiting. See `features/28`.
- [x] **Physical copy & content dedup** — rescue-copy with provenance, dedup reconciler, boomerang
  guard. See `features/11`.
- [x] **Storage quotas** — trigger-maintained counters, Redis fast path, daily reconcile, admin UI. See
  `features/22`.
- [x] **Resolver admin dashboard** — delegation-token auth, `/api/resolver-admin/*`, placement
  strategies. See `features/23`, `features/24`.
- [x] **Registration rules** — open/invite/admin-invite modes, invite codes, invite graph.
- [x] **Admin config instead of envs** — unified `common::settings` engine + metadata-driven
  `SettingsPanel` across backend/worker/resolver.
- [x] **Resolver chore** — router under one `/archypix-resolver/` prefix; `/info` bootstrap discovery.
  See `features/25`.
- [x] **Picture creator** — owner-vs-creator attribution, propagated and locally overrideable; later a
  rule field and part of batch view/editing. See `features/26`.
- [x] **Public shares** — link-gated pull shares with anonymous contribution and authenticated convert,
  reusing the app gallery. See `features/27`. *Follow-up:* cross-instance save-a-copy, Convert +
  share-back UI, full upload dialog on the public page, sort/filter on the public listing.
- [x] **Query presence filters & proximity sorts** — `present|missing` filters, `has_gps`, directed
  bracketing, haversine time/geo proximity sorts. See `features/29`.
- [x] **Photos fix tools** — guided GPS/capture-date fix modes with suggestions, interpolation and bulk
  preview. See `features/30`. *Deferred:* date run-interpolation, null-island heuristic, batched-propose
  endpoint.
- [x] **Tag metadata** — decorative `tag_metadata` side table (display names, colour, cover, date
  range, order, per-tag view prefs, `webdav_dir_name`); engine-invisible except WebDAV naming and
  empty-directory existence. One enriched `GET /tags` payload behind a 60 s cache, a debounced
  client write queue, the edit/new-tag dialogs, share badges, reorder mode and drag-to-tag, and the
  sender's name/description/cover carried across a share. See `features/34_tag_metadata.md`.
  *Deviations + deferred:* `features/34 §16` — notably the cover picker and the per-tag view controls
  (`view_mode`/`grouping`/`subtag_placement`), which are stored and served but wait on feature 35.
- [x] **Frontend fixes** — `dark:` variant keyed off the in-app `.light` class, not
  `prefers-color-scheme`.
