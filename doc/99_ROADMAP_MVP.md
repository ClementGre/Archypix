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
- [x] **Federation robustness** — read paths degrade instead of 500ing on a down peer
  (`PictureListItem.owner_reachable`, single-picture `503`); one typed versioned `POST /api/federation/message`
  envelope replacing the eight verb routes (per-message exact-match `VERSION` → `426`), collapsed into a single
  generic `FederationClient::send`; three distinct outbound timeouts (bounded client) + relative-TTL grant +
  proactive token refresh + single-flight handshake + backend-URL bust-on-failure + serve-stale-on-resolver-outage;
  deliver-then-commit accept/reject + `claim` idempotency; stale-announcement guard (`pictures.remote_updated_at`);
  presign expiry threaded into a truthful cache TTL; per-peer/-IP rate limiting with a Redis recent-rejections store
  + `GET /api/admin/rate-limits`; **deleted** the dead `federation_messages` table + `domain::federation` types +
    `PictureEditRequest.idempotency_key`. **Frontend** (§13): owner-offline tile with retry on
    `owner_reachable === false`, presigned-URL auto-refresh on expiry/`403` (grid/lightbox/sidebar), admin
    "Rate limiting" tab (limits `SettingsPanel` + recent-rejections timeline + attack flag); share-action `503`/`426`/`4xx`
    reasons surface through the backend's tailored messages via the existing `apiErrorMessage` toasts. See
    `doc/features/28_federation_robustness.md`.

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
- [x] **Public shares** — link-gated *pull* shares served by the owner backend (live coverage, no
  `IncomingShare`): unauthenticated view + anonymous contribution (`creator = #name`, dedup-rejected),
  authenticated convert (subscribe to a **derived** share via the recipient-initiated
  `shares/public/claim` verb + save-a-copy). One `allow_originals` tier vs a view-only gallery
  (thumbnails only, EXIF/GPS stripped from bytes **and** JSON). Backend (`public_shares` table +
  `TokenType::PublicShare` + `/api/public/shares/*` view/contribute + `/api/authenticated/shares/public`
  management/convert + the claim verb + tag-rename cascade + tests) and frontend (`/s/:domain/:user/:token`
  public gallery reusing the lightbox/detail leaves, contributor upload, convert menu, and the owner's
  "Public share links" manager). Depends on feature 26. See `doc/features/27_public_shares.md`.
  **Review pass (2026-07):** fixed the cross-instance Subscribe 404 (`receive_public_claim` now resolves
  the requester via `find_local_user_id`, not a bare owner-side username lookup, and only rejects a genuine
  self-subscribe); the owner can no longer save-a-copy/subscribe from their own album (backend guard + the
  frontend hides the actions). Frontend **factorized to reuse the app gallery**: the public page now shares
  `PhotoCard` (aspect-ratio grid + app selection semantics), the `Lightbox`/`LightboxCarousel` (via a new
  read-only `PictureSource` context), `SidePanel` (resizable/mobile details), the `TopBar` chrome, and a
  factored `ThumbnailSizeSlider` footer; `PublicShareDialog` is now create-**or**-edit with an info popover,
  friendlier visitor terms, and a tag-tree "New public share link…" entry. **Follow-up:** cross-instance
  save-a-copy (§10); Convert + share-back UI; reuse the full `UploadDialog` and read-only `ExifInlineEditor`
  on the public page; sort/filter on the public listing.
- [x] **Picture creator better integration** — creator is now a **rule field** (`Field::Creator` over the
  resolved displayed creator, feature 13 §2.2; `PipelineInput.creator` resolved in pipeline evaluation;
  frontend `RULE_FIELDS` Ownership entry) and part of **batch view/editing**: the aggregate summary
  carries a resolved-creator distinct `FieldAggregate` (`PictureRepository::aggregate_creator`), and
  `PATCH /pictures/creator` (`batch_set_creator_selection`) sets it over a selection (owned →
  authoritative `creator` re-announced, received → `creator_override`, `dry_run` breakdown). Creator
  resolution refactored into reusable free fns in `domain::picture`. Frontend `BatchCreatorControl` +
  `useBatchMutations().creator`. Also added a companion **`owner`** rule field (resolved owner identity
  `@user:domain`, a string comparison alongside the `is_owned` boolean); both `owner` and `creator` are
  **non-nullable** and drop the `is set`/`is not set` operators (frontend `FieldDef.nullable`, backend
  rejects `is_present`). See `doc/features/26_picture_creator.md §11` and `13_better_rules.md §2.2`.
- [x] **Frontend fixes** — `dark:` tailwind variant redefined via `@custom-variant` (`index.css`) to key off the in-app `.light` class on `<html>`
  instead of the browser's `prefers-color-scheme`.
- [x] **Better trash** — trash is now a **filter over the main view**, not a separate page. Backend `PictureListFilter.trash`
  (`TrashFilter::Exclude|Include|Only`, wire `?trash=exclude|include|only`) threads through the list / hierarchy `browse` / selection
  (batch) filters; `push_filters` gained the `Only` (trashed-only) arm. Frontend replaced the `/trash` page + `deleted=1` param with a
  three-state grid-header `TrashToggle` (Photos / All / Trash → `trash` URL param), removed the "Include trashed" checkbox, and the main
  gallery (metadata, tag filtering, batch restore in the selection panel) now serves trashed pictures directly — no client-side `deleted_at`
  filtering. Profile "Open trash" deep-links to `/?trash=only`.
- [x] **Query presence filters & proximity sorts** — reusable per-field `gps`/`capture_date` presence filters
  (`present|missing`) + `missing_any` OR (mutual-exclusion 400) + `has_gps` list field + directed bracketing lookup +
  time/geo proximity sorts (`sort=time_near|geo_near` + `near_*`, required-param 400, `id` tiebreak, **haversine**
  geo ordering — antimeridian-safe, no index/PostGIS). Threaded through the flat list, hierarchy `browse`, and
  feature-14 selections. `PictureListItem` also carries a geo-sort-only `distance_m` (Rust haversine over the page).
  **Frontend**: `useGalleryParams` presence/proximity URL state, grid-header **Issues filter** (All / Missing GPS /
  Missing date / Any issue), `SelectionPanel` "Nearby in time/place" actions, `FilterControls` proximity indicator +
  clear, per-tile distance badge. Substrate for the fix tools. See
  `doc/features/29_query_proximity_and_missing_filter.md`.
- [x] **Photos fix tools** — guided GPS/capture-date fix modes: highlight-in-context, filename/source-file/ingested
  date suggestions, grid-local GPS interpolation (directed-bracket fallback), explicit target→references selection,
  bulk preview; received pictures (local override / propose). **Backend**: `pictures.original_file_created_at`
  (migration `0012`, source file creation time via WebDAV `X-OC-CTime`, on list/detail) + `undated_first` date-fix
  ordering. **Frontend**: `fix` param enabled from `IssuesFilter`, `photos/fix/*` (fix section in the details panel +
  `GpsFixPanel`/`DateFixPanel`/`FixBulkDialog`/`FixBulkSection`/`ReferencePreview`/`ReferenceBar`),
  `lib/filenameDate`/`gpsInterpolation`/`fixBulk`/`dateSuggestions`, date chips in the normal editor. Depends on feature 29. See
  `doc/features/30_photos_fix_tools.md` (supersedes the feature 21 stub).
  *Deferred:* date run-interpolation, null-island `(0,0)` heuristic, batched-propose endpoint, frontend test runner.
- [ ] **Versioning better support** — presign and CRUD on versions; frontend viewing and editing.
- [ ] **ML workers** — `ml_style`, `ml_people`, `ml_group_location` handlers; per-user ML snapshots in MinIO.
- [ ] **EXIF edit history** — per-picture metadata revision history for review/undo.
- [ ] **Advanced WebDav** — directory-level DELETE/MOVE/COPY, conditional/range requests, real LOCK/UNLOCK.
- [ ] **Visual picture editing** — crop, brightness/contrast, resize in `edit_picture` worker.
- [ ] **Rate limiting & validators** — more rate limiting, real structured framework allowing to list the rate limiters in the admin dashboard with
  the window size + limit within window.
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
