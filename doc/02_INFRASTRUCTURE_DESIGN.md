# Infrastructure Design

- Resolver (Rust service)
    - Purpose: map username → owning backend domain (implements WebFinger). Enables multiple backends to share one global identity domain.
  - Roles:
      - WebFinger endpoint: answer `/.well-known/webfinger` requests with the resolved backend URL.
    - User registration routing: `POST /api/public/register` (the same path the standalone backend serves, so the frontend uses one URL across
      topologies) — picks least-loaded backend, forwards registration, stores `username → back_domain` mapping.
      - Backend self-registration: `POST /api/backends` — backends call this at startup; the resolver stores `back_domain`, `use_https`, and
        `internal_url`.
      - Mapping update: `POST /api/update` — called by backends when a user migrates to another instance.
    - Key env vars: `GLOBAL_DOMAIN`, `RESOLVER_JWT_SECRET`, `DB_HOST/DB_USER/DB_PASSWORD/DB_NAME`.

- Backend (Rust backend instance, per domain)
    - Purpose: authoritative per-instance application server and metadata store.
    - Roles:
        - HTTP API & WebDAV: serve user requests, uploads, sync client endpoints.
        - WebFinger client: cross-instance discovery; caches backend base URLs in Redis (`WEBFINGER_USE_HTTPS` controls the scheme).
        - Postgres: authoritative metadata (users, pictures, tags, shares, jobs). Key picture columns: `file_hash` (SHA-256, WebDAV ETag),
          `file_size`. On a presigned upload the backend reads the authoritative `file_size` from S3 (`HEAD`) rather than trusting the client; the
          client's SHA-256 (computed the same way as the worker) is stored as a provisional `file_hash` and re-confirmed by `gen_thumbnail`.
      - Federation endpoints: handle inbound/outbound federation messages (share announce/revoke, presign requests).
        - Job queue owner: writes `pending` jobs; exposes `/api/worker/*` for workers to claim/complete. Issues a one-time `claim_token` per claim.
        - In-process task queue (`infra/tasks.rs`): DB-only async tasks (tag-rename cascade, pipeline evaluation).
        - Recurring scheduler (`infra/scheduler.rs`): job watchdog (resets stale `processing` jobs, default 600 s timeout), job cleanup (prunes
          terminal
          jobs), pipeline recovery sweep, trash purge sweep (physically deletes owned pictures past their `trash_retention_days`).
        - Redis: sessions, presigned URLs, federation tokens, backend domain mappings.

- Workers (`archypix-worker`, one or more Rust processes)
    - Purpose: perform CPU/GPU-intensive work; never access the database or S3 directly.
  - Job transport: **Postgres-backed queue** (`SELECT FOR UPDATE SKIP LOCKED`). Workers poll `GET /api/worker/jobs/next`; the backend returns a job
    with presigned S3 URLs and a one-time `claim_token`. Workers echo the token in every `complete`/`fail`; mismatches are rejected (409).
  - S3 access: exclusively via presigned URLs.
  - Auth: short-lived JWT (`WORKER_JWT_SECRET`), cached in-process.
    - Implemented job types:
        - `gen_thumbnail` — download original, compute `file_size` + SHA-256 `file_hash`, extract EXIF, generate small/medium/large WebP thumbnails
          (skipped, not failed, for non-thumbnailable formats so size/hash are still reported), upload, report
          `exif`/`blurhash`/`file_size`/`file_hash`/`thumbnails_generated` to backend.
        - `edit_picture` — download original, apply EXIF overrides, compute `file_size`/`file_hash`, upload, optionally regenerate thumbnails.
  - Stub job types (infrastructure ready, not yet implemented): `ml_style`, `ml_people`, `ml_group_location`.
  - Completion: backend applies picture updates + marks job done in one transaction. Auto-retries up to `max_retries` (default 3) on failure.

- MinIO (S3-compatible object storage)
    - Purpose: durable blob store for original images, derivatives, version snapshots, and exports.
  - Buckets: staging (short-lived; auto-expires via lifecycle rule), pictures (the current/latest
    file — mutable: overwritten in place on edit), versions (previous versions plus the preserved
    original), small/medium/large (thumbnails).
      - S3 keys derived deterministically (never stored): `{user_id}/{picture_id}` for originals/thumbnails; `{user_id}/{picture_id}/{version_id}` for
        versions.
      - Three S3 endpoint slots: `S3_ENDPOINT` (server-side), `S3_PUBLIC_ENDPOINT` (browser presigns), `S3_WORKERS_ENDPOINT` (worker presigns,
        defaults to `S3_ENDPOINT`). Needed when internal/external Docker addresses differ.

- Frontend (static CDN + clients)
    - Single static site served from CDN; no per-instance build.
    - Discovery: resolve `@username:domain` → backend URL via WebFinger before making API calls.
    - All API and WebDAV calls go to the resolved backend for that user.

**Invariants**

- Each backend is authoritative for its users (Postgres is the single source of truth per instance).
- Workers publish results; backends persist — workers never write to backend databases or S3 directly.
- All persistent storage uses the **global domain**. Backend domains are resolved on demand via WebFinger and cached in Redis.
- Job queue transport is Postgres (`SELECT FOR UPDATE SKIP LOCKED`). Workers are stateless HTTP clients.
- The `claim_token` protocol prevents stale workers from overwriting the results of a re-claimed job.
