# Archypix — Worker

The image-processing worker for Archypix. It polls the backend for pending jobs, processes pictures
(thumbnails, EXIF extraction, file hashing, BlurHash), and reports results back — without ever
touching the database or S3 directly.

For a full overview of the project, see the [root README](https://github.com/ClementGre/Archypix).

## Tech stack

| Concern             | Crate / tool                                                                   |
|---------------------|--------------------------------------------------------------------------------|
| Async runtime       | [Tokio](https://tokio.rs/)                                                     |
| HTTP client         | [reqwest](https://github.com/seanmonstar/reqwest)                              |
| Image processing    | [magick_rust](https://github.com/nlfiedler/magick-rust) (ImageMagick bindings) |
| EXIF extraction     | [rexiv2](https://gitlab.gnome.org/GNOME/gexiv2) (GExiv2 / Exiv2 bindings)      |
| BlurHash generation | [blurhash](https://github.com/nicowillis/blurhash)                             |
| File hashing        | [sha2](https://github.com/RustCrypto/hashes) (SHA-256)                         |
| Auth                | [jsonwebtoken](https://github.com/Keats/jsonwebtoken)                          |
| Structured logging  | [tracing](https://github.com/tokio-rs/tracing) + tracing-subscriber            |
| Health check server | [Axum](https://github.com/tokio-rs/axum)                                       |

## System prerequisites

The worker links against two native C libraries that must be installed before building:

### ImageMagick (for thumbnail generation)

```bash
# macOS
brew install imagemagick

# Debian / Ubuntu
apt-get install libmagickwand-dev

# Fedora / RHEL
dnf install ImageMagick-devel
```

Verify the installed version:

```bash
pkg-config --modversion MagickWand
```

The `magick_rust` crate version in `Cargo.toml` must match your system ImageMagick version.

### GExiv2 / Exiv2 (for EXIF extraction)

GExiv2 is a GLib/GObject wrapper around Exiv2.

```bash
# macOS
brew install gexiv2

# Debian / Ubuntu
apt-get install libgexiv2-dev

# Fedora / RHEL
dnf install gexiv2-devel
```

## Configuration

Copy `.env.example` to `.env`. The file is fully commented and lists all available variables with their defaults.

```bash
cp .env.example .env
```

Key variables:

- **`BACK_URL`** — Base URL of the Archypix backend (e.g. `http://backend:8000`).
- **`BACK_DOMAIN`** — The backend's public domain; used as the JWT audience. Must match the backend's `BACK_DOMAIN`.
- **`GLOBAL_DOMAIN`** — Shared identity domain; used in the JWT `instance` field. Must match the backend's `GLOBAL_DOMAIN`.
- **`WORKER_JWT_SECRET`** — Shared HMAC secret for signing worker JWTs. Must be identical to the backend's `WORKER_JWT_SECRET`.
- **`WORKER_ID`** — Unique name for this instance (defaults to a random short ID).
- **`POLL_INTERVAL_MS`** — How often to poll for new jobs when idle (default: `1000`).
- **`MAX_CONCURRENT_JOBS`** — Maximum jobs processed simultaneously (default: `6`).
- **`JOB_TYPES`** — Comma-separated list of accepted job types; empty means all (e.g. `gen_thumbnail,edit_picture`).

Log level:

```bash
RUST_LOG=info,archypix_worker=debug    # default
RUST_LOG=info,archypix_worker=trace    # verbose
```

## Building

Prerequisites: Rust (stable, edition 2024) via [rustup](https://rustup.rs/), plus the system libraries listed above.

```bash
# Development
cargo run

# Release
cargo build --release
./target/release/archypix-worker

# Docker
docker compose up
```

## Job types

### `gen_thumbnail`

Downloads the original picture (streaming, no full memory buffer), then:

1. Runs a MIME pre-flight check — rejects unsupported formats before downloading thumbnails.
2. Extracts EXIF metadata via rexiv2 (when `is_initial: true` in the job config).
3. Computes the SHA-256 `file_hash` and reads `file_size` from disk.
4. Generates WebP thumbnails at three sizes using ImageMagick.
5. Computes a BlurHash from the original image.
6. Uploads thumbnails via presigned PUT URLs.
7. Reports all metadata to the backend.

| Variant | Height  | Format |
|---------|---------|--------|
| small   | 150 px  | WebP   |
| medium  | 500 px  | WebP   |
| large   | 1000 px | WebP   |

Width is derived from the original aspect ratio. The `THUMBNAIL_VARIANTS` constant in
`imaging/resize.rs` is the single source of truth for these values. Encoded at quality 68
(`THUMBNAIL_QUALITY`, libwebp default is 75).

### `edit_picture`

Downloads the original picture, then:

1. Applies EXIF overrides into the file's embedded metadata via rexiv2.
   A write failure (unsupported format) is a **permanent error** — the job is immediately
   marked failed without retry. MIME screening will be added server-side in a future milestone.
2. Computes `file_hash` (SHA-256) and `file_size` from the modified file.
3. Uploads the modified file via the `output` presigned PUT URL.
4. Regenerates thumbnails and BlurHash if the backend provided thumbnail presigned URLs
   (only for visual transform jobs; EXIF-only edits skip this step).
5. Reports `file_size`, `file_hash`, and optionally `blurhash` to the backend.

Visual transforms (crop, resize, colour adjustments) are not yet implemented; the original is
uploaded unchanged when a `visual` config is present.

### ML jobs (stubs)

`ml_style`, `ml_people`, and `ml_group_location` are accepted but not yet implemented. The worker
logs the job type and immediately reports completion with an empty result.

## Claim-token protocol

The backend issues a one-time `claim_token` UUID when a job is claimed. The worker must include
this token in every `complete` and `fail` call. The backend rejects calls where the token does not
match or the job is no longer in `processing` state (returns 409). This prevents a stale worker
(reset by the backend watchdog) from corrupting the results of a re-claimed job.

## Code structure

```
src/
  main.rs              Entry point: loads config, starts health server + job loop.
  config.rs            Config::from_env(); all settings with defaults and validation.
  error.rs             WorkerError enum + is_retriable() (transient vs permanent).
  auth.rs              generate_token(): HS256 JWT generation.
  backend.rs           BackendClient:
                         api_http      — 10 s timeout, for claim/complete/fail API calls.
                         presign_http  — connect-only timeout, for large-file S3 transfers.
                         Token cache   — refreshes 30 s before expiry, shared across clones.
                         claim_next_job / complete_job / fail_job(claim_token)
                         download_presigned (streaming) / upload_presigned
  imaging/
    exif.rs            extract_exif() / write_exif_overrides() — rexiv2, blocking.
    hash.rs            hash_file() — SHA-256 hex in 64 KiB chunks, blocking.
    resize.rs          generate_thumbnail() (ImageMagick/WebP), generate_blurhash();
                       THUMBNAIL_VARIANTS: single source of truth for sizes.
    thumbnailer.rs     run(): spawn_blocking for CPU work, async upload per variant.
  jobs/
    mod.rs             run_job_loop(): semaphore-bounded poll (blocks on semaphore,
                       not sleep-poll); dispatch() threads claim_token to all handlers.
    thumbnail.rs       gen_thumbnail handler.
    edit_picture.rs    edit_picture handler.
    ml.rs              Stub handler for ml_* job types.
```

## Health check

A minimal HTTP server runs on `LISTEN_ADDR` (default `0.0.0.0:80`) and exposes:

```
GET /health  →  200  {"status": "healthy", "service": "archypix-worker"}
```

Use this endpoint for Docker/Kubernetes liveness and readiness probes.
