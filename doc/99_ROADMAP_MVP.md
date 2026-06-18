# Backend + Resolver MVP Roadmap

## Completed

- [x] Core infrastructure: layered Rust architecture (domain/repository/services/clients/api/infra), Axum router, SQLx migrations, AppState wiring (
  Postgres, Redis, MinIO, JWT, federation, resolver clients).
- [x] Auth, users, pictures, tags, shares, settings endpoints; federation auth handshake (request/grant) and share announce/revoke; resolver
  user-management endpoints.
- [x] Picture upload pipeline: presigned staging → server-side copy → optional versioning (S3 copy + DB record in one transaction, `version_id`
  matches S3 key).
- [x] Resolver self-registration and tests with a frontend.
- [x] Worker pipeline (foundation): Postgres-backed job queue, HTTP-only worker crate.
- [x] Tag sharing full support: accept flow, pictures announcement, same-backend short-circuit, received picture rows, `/SharedToMe/…` tag assignment,
  presign for same-backend and cross-instance received pictures.
- [x] Tests: domain unit tests, repository integration tests, service integration tests, worker HTTP contract tests, federation end-to-end and
  security tests.

## To-do for the MVP

- [x] **Tagging pipeline CRUD** — API to define tagging services (rules and segmentation).
- [x] **Tagging pipeline execution** — wire `services/tagging.rs` to run the domain pipeline evaluator on ingest/edit/share events; connect the
  in-process `TaskQueue::RunTaggingPipeline` variant.
- [x] **Tagging pipeline tags removal** — tags are stored per-source; pipeline tags are live and re-derived each run, with stale `rule`/`segment`/
  `share_mapping` tags removed atomically. Disabling a service drops its tags; deleting one promotes them to `manual` if `promoting=true`. Provenance
  is exposed per tag.
- [x] **Better sharing support** — per-picture token presign model replacing `OutgoingShare.share_token`; pipeline-driven announce/unannounce via
  `share_announcements` tracking table; ShareBack auto-accept with `SharedTagMappingService` rule creation; loop prevention (sender + recipient);
  transitive sharing and presigning end-to-end; token refresh on partial revocation; transitive revocation for `SharedToMe` re-shares; `SharedToMe`
  prefix protection. See `doc/features/01_better_sharing_support.md`.
- [x] **Exif edition** — write-through EXIF edit (single + batch) with `set`/`clear` semantics; the DB is updated synchronously and a worker
  `edit_picture` job reconciles the S3 original's embedded EXIF, with guaranteed convergence (value-gated revert on permanent failure, one in-flight
  reconcile per picture, MIME preflight → `unsupported`). EXIF edits re-dirty the pipeline and propagate gps/exif/orientation to federated recipients.
  See `doc/features/04_better_exif_support.md`. (History is a v1.0 item below.)
- [x] **Admin endpoints** — user list/suspend/delete, job status, instance metrics.
- [ ] **Full frontend** — v1 of a user-friendly frontend, with super simple code for a MvP, but with a realistic user experience that could give an
  idea of what the final front could look like.
- [x] **Hierarchies** — node-tree `config` model (mirror/query/static), pure validation, and the read
  resolver (`build_tree` + most-specific-wins per-directory `TagPredicate`); CRUD + `tree`/`browse`
  navigation endpoints; generalised `list_pictures` tag predicate plus flat `include_tags`/
  `exclude_tags`/`match`/`untagged` on `GET /pictures`. Write-back is modelled (op-lists, compliance,
  `safeDeleteMode`) so the schema is write-ready, but the **write endpoints ship with WebDAV**. See
  `doc/features/05_hierarchies.md`.
- [x] **WebDAV** — backend implemented: per-hierarchy encrypted token (HTTP Basic) mounted at
  `/webdav/{slug}`, protocol-agnostic `VirtualFs` over the hierarchy resolver, reads via presigned
  redirect or backend proxy, writes (PUT/DELETE/MOVE/COPY/MKCOL) as tag write-back with hash-dedupe
  identity and `pictures.file_hash` as ETag, frontend token dialog, and OS-junk filtering
  (AppleDouble `._*`, `.DS_Store`, …). See `doc/features/06_webdav.md`.
  Remaining WebDAV work (see `06_webdav.md` §21):
    - [x] **Versioning on overwrite** — an overwrite-PUT consults the user's `versioning_mode` and
      snapshots a `picture_version` before replacing the bytes (`none`/`original_copy`/
      `full_versioning`), reusing the worker edit path's snapshot machinery.
    - [x] **Streamed uploads** — the PUT body streams to a temp file (never buffered in memory), is
      hashed inline with the common crate's chunked `hash_file`, then streams to S3 via
      `Storage::put_object_file`; `WEBDAV_MAX_UPLOAD_BYTES` is now a real env config and a zero-byte
      PUT ingests nothing. A true multipart-to-S3 stream remains a later optimization.
    - [x] **Brand-new mirror subdir auto-tag (§9)** — a PUT/COPY/MOVE into a not-yet-existing path
      whose nearest ancestor is a `mirror` node mints the deepest tag from the path segments
      (validated as tag labels). MKCOL under a mirror records a transient Redis pending-dir marker
      so the empty directory shows in PROPFIND until a file lands and mints the real tag; OS-junk
      dotfiles are likewise stored as transient Redis sidecars that round-trip in listings (§11).
    - [x] **Case-insensitive write-side tag reuse (§10c)** — on write each assigned tag path is
      folded onto an existing case-variant tag (`reuse_existing_case`), so a case-insensitive client
      never mints a case-only-duplicate sibling.
    - [ ] **Directory-level operations** — DELETE/MOVE/COPY on a collection (whole directory) are
      not supported (only files); decide semantics (bulk re-tag vs reject) and implement.
    - [ ] **Conditional & range requests** — honor `If-Match`/`If-None-Match` (ETag) and the
      `Overwrite` header on PUT/MOVE/COPY; support HTTP `Range` in proxy-read mode.
    - [ ] **Real locking** — replace the fake advisory LOCK/UNLOCK with an enforced lock store
      (in-memory or Redis) if multi-writer correctness becomes a concern.
    - [ ] **Quota properties** — expose `quota-used-bytes`/`quota-available-bytes` in PROPFIND.
    - [x] **End-to-end VFS tests** — `back/tests/vfs.rs` drives the VFS against a seeded DB and an
      in-memory storage mock: list/stat/proxy+redirect reads, the full write taxonomy (PUT new/
      overwrite/dedupe/un-delete/empty, MOVE/COPY/DELETE in both delete modes, the 409 path),
      versioning-on-overwrite, and the §10c case-fold.
- [x] **Better workers** — `BACK_DOMAIN`/`BACK_URL` accept comma-separated lists; one poller loop per
  backend, all sharing a single global semaphore (`MAX_CONCURRENT_JOBS`, default `6`).
  Burst-friendly: a successful claim immediately re-polls without sleeping, so a batch upload saturates
  all slots as fast as the backend issues claims. Fair multi-backend allocation emerges naturally from
  semaphore competition — no explicit scheduler needed.
- [ ] **Security audit** — audit the code for security holes.
- [ ] **Trash & restore** — pictures deletion, announcement to sharing recipients setting their `deleted_at` too. Adding an endpoint allowing to copy
  the picture physically to keep it even if the owner trashed it.
- [ ] **Tag rename cascade** — expose API endpoint that triggers the in-process `TaskQueue::TagRename` task; add cascade to outgoing shares,
  segmentation configs, and hierarchies (currently only tags table is updated).
- [ ] **Federation robustness** — token refresh/rotation schedule, retry logic for failed announce/revoke, presigned URL caching for remote picture
  access.
- [ ] **Rate limiting and validators** — Redis-backed limits on auth, federation, and public endpoints; session invalidation on logout. Password,
  emails, usernames validators.

## To-do for the v1.0

- [ ] **ML workers** — implement `ml_style`, `ml_people`, `ml_group_location` job handlers; add per-user ML snapshot storage in MinIO.
- [ ] **Edit picture — visual edits** — add crop, brightness/contrast, and resize support to the `edit_picture` worker job.
- [ ] **Adavanced Frontend** — upgraded v2, or a v3 frontend with a more advanced user experience.
- [ ] **EXIF edit history** — persist a per-picture metadata revision history (dedicated store, not the jobs table) so EXIF edits can be reviewed and
  undone.
