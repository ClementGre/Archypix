# Worker Architecture

Workers are standalone Rust processes (`archypix-worker`) that poll the backend for jobs over HTTP and never touch the database or S3 directly. See
`03_BACKEND_ARCHITECTURE.md §5 worker endpoints` for the HTTP API.

## Module layout (`worker/src/`)

```
main.rs              — tokio entry-point; starts health server + one job-loop task per backend
config.rs            — Config::from_env(); BackendConfig (per-backend URL/domain); shared settings
auth.rs              — generate_token(): HS256 JWT generation (per-backend aud); cached via BackendClient
error.rs             — WorkerError; is_retriable() classifies transient vs permanent failures
backend.rs           — BackendClient (one per backend): two separate HTTP clients (api_http 10 s timeout,
                       presign_http connect-only timeout for large-file transfers);
                       per-instance JWT token cache (refreshed 30 s before expiry);
                       claim_next_job / complete_job / fail_job /
                       download_presigned (streaming) / upload_presigned

jobs.rs              — run_job_loop(): shared semaphore → poll → spawn; dispatch()
jobs/thumbnail.rs    — gen_thumbnail: download → file_size + hash → content_hash → EXIF → thumbnails
                       (only if the MIME is thumbnailable) → complete. A non-thumbnailable format is
                       not an error: it still reports size/hash and completes with thumbnails skipped,
                       so every ingested picture gets an ETag/size even without a thumbnail.
jobs/edit_picture.rs — edit_picture: download → EXIF set/clear write → thumbnail regen (visual) →
                       hash → upload original (last fallible step) → complete. The DB is updated
                       synchronously at edit time (write-through); this job only reconciles the S3
                       original's embedded EXIF to match. Uploading the original last preserves the
                       file-untouched-on-failure invariant the backend's revert depends on.
jobs/ml.rs           — stub for ml_* jobs (log + complete with empty result)

imaging/exif.rs      — extract_exif() / write_exif_overrides(set, clear) (rexiv2, blocking).
                       Full editable-field coverage on write (date, GPS, orientation, make, model,
                       focal length, f-number, ISO, exposure time) plus per-field clear (tag delete).
imaging/hash.rs      — hash_file(): SHA-256 hex digest in 64 KiB chunks (blocking)
imaging/content_hash.rs — content_hash(): SHA-256 over the image's metadata-stripped bytes (feature
                       11 §4) — strips JPEG APPn/COM and PNG text/time chunks, hashes the framing +
                       scan. Stable across EXIF edits, changes on a visual re-encode, deterministic
                       across instances. `None` for a format it can't strip (backend falls back to
                       file_hash). edit_picture recomputes it from the result so a visual edit regroups.
imaging/resize.rs    — generate_thumbnail() (ImageMagick/WebP), generate_blurhash(),
                       image_dimensions() (decoded raw pixel w/h — authoritative source of
                       pictures.width/height, EXIF only a fallback);
                       THUMBNAIL_VARIANTS const: single source of truth for sizes
imaging/thumbnailer.rs — run(): spawn_blocking for CPU work, async upload per variant
```

## Claim-token protocol

When a job is claimed, the backend generates a fresh `claim_token` UUID and stores it on the job row. The token is returned in `ClaimJobResponse`.
Every subsequent `complete` and `fail` call must include the same `claim_token`.

The backend's SQL guards `AND claim_token = $x AND status = 'processing'` on both UPDATE operations. If the watchdog resets a stale job (clearing
`claim_token`) and a second worker re-claims it, the first worker's late `complete` or `fail` call will find no matching row and receive a 409. This
prevents stale workers from corrupting re-claimed jobs.

## Job loop

One loop task runs per backend; all loops share a single `Arc<Semaphore>` bounded by
`MAX_CONCURRENT_JOBS`. When multiple backends are configured the loops compete fairly for slots:
a backend with many pending jobs saturates its share; a quiet backend yields without any explicit
scheduler.

```
// per-backend loop
loop {
  sem.acquire_owned().await           ← blocks until a global slot is free
  claim_next_job():
    None  → drop permit, sleep poll_interval_ms
    Some  → tokio::spawn dispatch(job) (permit dropped when task exits)
            // no sleep — immediately compete for the next slot (burst-friendly)
    Err   → drop permit, sleep 5 × poll_interval_ms
}
```

When a job is claimed the loop immediately re-enters (no sleep), so a burst of pending jobs
saturates all `MAX_CONCURRENT_JOBS` slots as fast as the backend can issue claims. When idle the
loop backs off to `poll_interval_ms` to avoid hammering the backend.

## Error policy

Some errors are transient and can be retried, others are permanent and should be marked `failed` permanently. `is_retriable()` on `WorkerError`
classifies them. On back, the watchdog (`infra/job_watchdog.rs`) runs every `JOB_WATCHDOG_INTERVAL_SECS` (default 60 s) and resets jobs stuck in
`processing` for longer than `JOB_PROCESSING_TIMEOUT_SECS` (default 600 s) by incrementing `retry_count` and returning them to `pending` (or `failed`
if retries exhausted). It also clears `claim_token` on reset.

## EXIF edit write-through

The backend applies EXIF changes to `pictures` synchronously; an `edit_picture` job reconciles the embedded EXIF in the S3 original.

- **Versioning predicate** (evaluated at job claim, `api/worker/handlers.rs`): `None` → never; `OriginalCopy` → snapshot on first edit only;
  `FullVersioning` → first edit or any visual edit (exif-only edits never add a version).
- **Convergence / revert**: on completion the backend flips to `synced` if the DB still matches the job's target, else enqueues a follow-up reconcile.
  On permanent failure it reverts the DB row to the job's `previous` snapshot — safe because uploading the original is the last fallible step, so
  failure never overwrote the file.

## Shared types (`archypix-common`)

Library crate shared between `back/` and `worker/` so wire shapes never drift:

| Module           | Key types                                                                                                                                                                                                                                        |
|------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `job.rs`         | `JobType`, `JobConfig`, `GenThumbnailConfig`, `EditPictureConfig`, `ExifEdit` (`set`/`clear`/`previous`, all `FullExif`), `ExifField`, `CameraExif`, `FullExif` (promoted + `camera`), `ExtractedExif` (`width`/`height` + flattened `FullExif`) |
| `transfer.rs`    | `ClaimQuery`, `ClaimJobResponse` (+ `claim_token`), `PresignedWrites`, `CompleteJobRequest` (+ `claim_token`, `file_size`, `file_hash`, decoded `width`/`height`), `FailJobRequest` (+ `claim_token`)                                            |
| `mime.rs`        | `MIME_TYPES_EXIF`, `MIME_TYPES_THUMBNAIL`, `supports_exif()`, `supports_thumbnail()`                                                                                                                                                             |
| `serde_utils.rs` | `csv` serde module for comma-separated `Vec<T>` query params                                                                                                                                                                                     |
