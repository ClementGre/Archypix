# Roadmap

One line per item. Details live in the matching `doc/features/NN_*.md`.

## Next

-    **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots.

## Toward a real product (adoption, mobile, hosting)

-    **Onboarding & opinionated defaults** — fresh instance auto-organizes out of the box, with progressive disclosure and a user-facing terminology pass.
-    **Bulk library import** — Google Takeout + local-folder import.
-    **External shared-album import** — one-time import of Google Photos and public iCloud Shared Albums as synthetic received shares; perpetual auto-bridge is demand-driven (ToS-fragile).
-    **PWA viewer** — mobile view / receive / organize without a native app.
-    **iOS background uploader** — reliable camera-roll backup (Android is covered by WebDAV).
-    **Backup & recovery discipline** — S3 versioning + replication the backend cannot delete from, Postgres PITR, purge-sweep guardrails. Prerequisite before hosting paying users.
-    **Managed hosting (archypix.com)** — managed instances; base + per-GB pricing with a ceiling. Needs a legal entity.

## Later

-    **Versioning better support** — presign and CRUD on versions; frontend viewing and editing.
-    **EXIF edit history** — per-picture metadata revision history for review/undo.
-    **Advanced WebDAV** — directory-level MOVE/COPY across parents, conditional/range requests, real LOCK/UNLOCK. Collection `MOVE` is `405` except for an in-place rename of a still-empty `show_when_empty` directory, which rewrites `webdav_dir_name` (Finder's create→rename flow, feature 34 §8).
    -    **Directory CTag** — derived per-collection change token; must key off `updated_at`, not `file_modified_at` (feature 32).
-    **Visual picture editing** — crop, brightness/contrast, resize in the `edit_picture` worker.
-    **Rate limiting & validators** — structured framework, limiters listed in the admin dashboard with window size + limit.
-    **Trace sampling + OpenTelemetry Collector** — deployment hardening. See `features/16_trace_sampling_and_collector.md`.
-    **Split `back` into crates** — extract `domain` (pure) and `repository` (95% of the query macros) so an `api`/`services` edit re-expands none of them, and the §B layer table is enforced by the compiler rather than by prose. The `routines` extraction was the prerequisite.
-   [~] **Video & audio playback** — Tier 1 done (inline `@vidstack/react`); Tier 2 ffmpeg transcode worker todo; Tier 3 HLS later. See `05_FRONTEND_ARCHITECTURE.md §9`.

## Known issues

-    A reconcile already claimed when an external (WebDAV) overwrite lands still writes its older target onto the new file (31 §5, narrowed by 33 §6.4).
-    A batch EXIF dry run previews a video under `edited` and counts it under `synced` — both read the stored status, but a video only earns its verdict once a write is attempted (33 §10).
-    A physical copy inherits a `pending` source status without an accompanying reconcile job, so it can sit in `pending` until the user resyncs (33 §4.1).

## Done

-    **Core infrastructure** — layered Rust architecture, Axum router, SQLx migrations, AppState.
-    **Auth, users, pictures, tags, shares, settings, admin** — plus the federation auth handshake and resolver user-management endpoints.
-    **Upload pipeline** — presigned staging → server-side copy → optional versioning.
-    **Resolver self-registration**.
-    **Worker foundation** — Postgres-backed job queue, HTTP-only worker crate.
-    **Better workers** — multi-backend support, global semaphore, burst-friendly polling.
-    **Tests** — domain unit, repository/service integration, worker HTTP contract, federation end-to-end and security.
-    **Tagging pipeline** — CRUD, event-driven execution, per-source tag lifecycle.
-    **Better sharing** — per-picture token presign, announce/unannounce, ShareBack, transitive sharing. See `features/01`.
-    **Shares name and message** · **incoming-share detail enrichment** — propagated to the recipient; richer frontend share popovers.
-    **EXIF editing** — write-through single + batch with convergence guarantees. See `features/04`.
-    **Robust EXIF sync** — `file_exif` snapshot, claim-time target, `write_failed` + manual Retry/Revert-to-file. See `features/31`.
-    **EXIF read path & engine fallback** — ExifTool dispatch + fallback, observed read states, one `JobOutcome`/`JobProduct` response endpoint. See `features/33`.
-    **Hierarchies** — node-tree config, read resolver, CRUD + `tree`/`browse`. See `features/05`.
-    **Hierarchy improvements** — `drop` nodes, per-node write-back tri-state, mirror depth/excludes. See `features/18`.
-    **WebDAV** — per-hierarchy endpoint, proxy reads, tag write-back, hash-dedupe, atomic-save staging. See `features/06`, `features/08`.
-    **WebDAV last-modified** — `getlastmodified` follows the bytes, not a re-tag. See `features/32`.
-    **Trash & restore** — soft delete, owner-deletion propagation, retention setting. See `features/09`.
-    **Better trash** — trash as a filter over the main view (`?trash=exclude|include|only`), no separate page.
-    **Recipient EXIF editing** — per-share `allow_exif_edit`, `local`/`propose` modes. See `features/10`.
-    **Admin endpoints** — user management, job status, instance metrics.
-    **Security audit** — security, privacy, reliability fixes. See `features/07`.
-    **Logging robustness** — span tracing, OTel compatibility, bounded operation-name cardinality. See `features/12`.
-    **Hash & size reliability** — authoritative S3-read `file_size`, worker-confirmed `file_hash`, debounced pipeline wakes.
-    **Better rules** — structured JSONB predicate tree, full field coverage, frontend `PredicateBuilder`. See `features/13`.
-    **Multi-picture edits** — `PictureSelection`/`PictureFilter`, batch aggregate/tags/EXIF/trash with dry-run. See `features/14`.
-    **Dedup at upload time** — batch presign hashes up front, dedupes within-batch and against existing/trashed pictures.
-    **Calendar segmentation & unified service config** — Calendar partition operator, unified `tagging_services.config`. See `features/20`.
-    **Tag rename cascade** — `POST /tags/rename` across manual tags, shares, pipeline and hierarchy configs.
-    **Federation robustness** — degrading read paths, one typed versioned message envelope, outbound HTTP hardening, rate limiting. See `features/28`.
-    **Physical copy & content dedup** — rescue-copy with provenance, dedup reconciler, boomerang guard. See `features/11`.
-    **Storage quotas** — trigger-maintained counters, Redis fast path, daily reconcile, admin UI. See `features/22`.
-    **Resolver admin dashboard** — delegation-token auth, `/api/resolver-admin/*`, placement strategies. See `features/23`, `features/24`.
-    **Registration rules** — open/invite/admin-invite modes, invite codes, invite graph.
-    **Admin config instead of envs** — unified `common::settings` engine + metadata-driven `SettingsPanel` across backend/worker/resolver.
-    **Resolver chore** — router under one `/archypix-resolver/` prefix; `/info` bootstrap discovery. See `features/25`.
-    **Picture creator** — owner-vs-creator attribution, propagated and locally overrideable; later a rule field and part of batch view/editing. See `features/26`.
-    **Public shares** — link-gated pull shares with anonymous contribution and authenticated convert, reusing the app gallery. See `features/27`. *Follow-up:* cross-instance save-a-copy, Convert + share-back UI, full upload dialog on the public page, sort/filter on the public listing.
-    **Query presence filters & proximity sorts** — `present|missing` filters, `has_gps`, directed bracketing, haversine time/geo proximity sorts. See `features/29`.
-    **Photos fix tools** — guided GPS/capture-date fix modes with suggestions, interpolation and bulk preview. See `features/30`. Null-island `(0,0)` is dropped at the read path (both engines + the received merge) and rejected on write, rather than the planned client heuristic — such pictures now read as plainly missing GPS. *Deferred:* date run-interpolation, batched-propose endpoint, a GPS-accuracy field (`GPSHPositioningError`).
-    **Tag metadata** — decorative `tag_metadata` side table (display names, colour, cover, date range, order, per-tag view prefs, `webdav_dir_name`); engine-invisible except WebDAV naming and empty-directory existence. One enriched `GET /tags` payload behind a 60 s cache, a debounced client write queue, the shared edit/create dialogs (a new tag is configured where it is created, from the tree or from the picker's *Customize*), share badges, reorder mode and drag-to-tag, and the sender's name/description/cover carried across a share. See `features/34_tag_metadata.md`. *Deviations:* `features/34 §16`. The cover picker and the per-tag view controls landed with feature 35.
-    **Timeline view** — structured browse over any tag subtree: one recursive rule (direct photos in grouping sections + a block per subtag), a **View** dropdown, Sort and Group by merged into one two-column menu, a `GroupedGridContext` giving one flat visible order across nested sections, and the gallery filter-param rewrite (`tag` + `inc`/`exc`; `exa` gone, `exact` single-valued on the wire). See `features/35_timeline_view.md`. *Deviations + deferred:* `features/35 §13`.
-    **Frontend fixes** — `dark:` variant keyed off the in-app `.light` class, not `prefers-color-scheme`.
-    **Backend build & layering chore** — `SQLX_OFFLINE` in `.cargo/config.toml` and `debug = "line-tables-only"` on `profile.dev` (incremental `cargo check` 13.8 s → 2.6 s, full rebuild 30 s → 25 s); `infra/routine/` extracted to a top-level `routines/` above `services`, with trigger payloads in `domain/routine.rs` and `RoutineHandle` taken from `archypix_common::routine`, making the layer graph acyclic.
-    **Backend file & test layout** — the 8 `#[sqlx::test]` modules left in `src/` moved to `back/tests/` (new `repository` suite) and now seed through `common`, dropping 8 duplicate `MIGRATOR`s, 6 `seed_user` and 3 `seed_picture`; genuine unit tests stay in-file (see 03 §I *Where tests go*). `repository/picture.rs`, `services/pictures.rs`, `services/vfs.rs` and `services/hierarchy.rs` split into submodules — largest backend file 2503 → 914 lines. Compile times unchanged (same crate); this is navigability only.
