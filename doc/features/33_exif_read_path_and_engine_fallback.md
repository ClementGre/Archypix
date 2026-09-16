# Feature 33: EXIF Read Path — Engine Fallback & Observed Sync States

## 1. Overview & Goals

Feature 31 made the **write** direction (DB → file) state-based and auditable. The **read** direction
(file → DB) was left at one `warn!`: an extraction failure logs and completes the job, so the picture
keeps the schema default `synced` — a claim that the DB matches a file we never read.

Three consequences, all silent:

1. "This file carries no EXIF" and "we could not read this file" are byte-identical rows.
2. A transient tool/IO outage during an ingest burst strips EXIF from everything in that window,
   with no retry, no record and no signal.
3. `file_exif` stays NULL, which disables header Revert and turns per-field reverts into clears —
   behaviour tuned in 31 §4 for a state the read path produces without recording why.

This feature closes that, and fixes the cause of most read failures while it is there: metadata is
read by **one** engine (rexiv2) while BMFF containers are *written* by **another** (ExifTool), so a
file ExifTool can handle is discarded because rexiv2 could not open it.

Goals:

- A second read engine, so a read failure is a two-engine verdict rather than one library's opinion.
- Every `exif_sync_status` value set by something **observed**, never by a schema default or a
  lookup table.
- The read direction represented in the state machine, with the same retriable/terminal
  classification the write path already has.
- Delete the derived guards this replaces (`thumbnails_generated_at` as an extraction proxy; the
  MIME allowlists threaded into set-based SQL).

Non-goal: mirroring the write path's machinery onto ingestion. A picture must not fail to ingest
because its EXIF block is corrupt — the thumbnail, hash and content-hash work in that job stays.

## 2. Decisions

1. **Dispatch by MIME first, fall back on failure.** BMFF (`image/heic|heif|avif`) reads go to
   ExifTool directly, mirroring `use_exiftool_for_write`; every other format tries rexiv2 and falls
   back to ExifTool only when rexiv2 fails. Reads and writes then use the same engine for the format
   class where writes already use ExifTool.
2. **rexiv2 stays primary.** ExifTool is a single `-stay_open` child behind a `Mutex`
   (`exiftool-0.3.1`), so making it the only engine turns metadata reads into a global serialization
   point across `max_concurrent_jobs` (default 6), with head-of-line blocking on large files. See §3.4.
3. **Both engines must produce identical `FullExif`.** Enforced by a differential test, not review (§3.3).
4. **The status column records observations only, and has no default.** Every insert path sets it
   explicitly (§4.1) — a default would assume an extraction follows, which three of the four insert
   paths do not do.
5. **`unsupported_mime` is a *write*-capability verdict, not a read one.** Videos are read (ffprobe)
   but cannot be written, so read-capability and write-capability are different facts; the status
   tracks sync work, and write-capability is a property of the MIME (§4.3).
6. **The write-capability verdict is stored, not derived.** Deriving it means an allowlist change instantly
   relabels never-read files as `synced` — the original bug in a new costume. A stored value is
   stale but *findable*, which is what an allowlist change actually needs, because the work it
   creates is re-reading, not relabelling (§8).
7. **Two `unsupported` values**, because they have different re-check triggers (allowlist growth vs.
   engine upgrade) and different user-facing meanings ("n/a" vs. "unreadable").
8. **Re-extraction is a separate action from resync**, and destructive — it makes the file
   authoritative (31 §5), so it must refuse rows with unsynced DB edits.

## 3. Engine layer

### 3.1 Dispatch and fallback

```
read_metadata(path, mime):
    if use_exiftool_for_read(mime):        # BMFF — same predicate shape as use_exiftool_for_write
        exiftool_read(path)
    else:
        rexiv2_read(path).or_else(|e| match e {
            UnsupportedFormat(_) => exiftool_read(path),   # second opinion
            other                => Err(other),            # IO/tool faults are not file verdicts
        })
```

Only a *format verdict* triggers the fallback. A retriable error is propagated as-is: falling back on
an IO error would convert a transient fault into a file verdict, which is the bug this feature exists
to remove.

`ToolUnavailable` from the fallback is **not** a file verdict either. `rexiv2 failed AND exiftool
unavailable` is retriable, so the job fails and retries; it must never land as `unsupported_file`.

### 3.2 The ExifTool reader

New `exiftool_read` in `worker/src/imaging/exif.rs`, written as feature 19 §5 step 2 (`read_metadata`
over `exiftool -json`) so the remaining feature-19 decision is a switch, not a rewrite. Maps onto the
existing `ExtractedExif`/`FullExif`/`CameraExif` shape via the crate's `json(path, &[…])`.

### 3.3 Parity — the normalization points

The differential test asserts equality of the **mapped `FullExif`**, not of raw tags. Four places the
engines cannot match without an explicit decision:

| Field | rexiv2 | ExifTool | Decision |
|---|---|---|---|
| Exposure time | rational `num`/`den` | `-n` gives `0.008` | Read `ExposureTime` **without** `-n` (`"1/125"`) and parse the fraction — never rationalize a float |
| GPS lat/lng | decimal via `get_gps_info()` | `48 deg 51' 29.60" N` unless `-n` | `-n`; apply S/W sign |
| GPS altitude | gated on tag presence, cast `i32` | needs `GPSAltitudeRef` for below-sea-level | Reproduce the presence gate and the sign |
| Capture date | explicit 5-tag priority chain (`exif.rs`) | composite `DateTimeOriginal`, own rules | Reproduce the chain explicitly; do not use the composite |
| Orientation | filtered to `1..=8` | description string unless `-n` | `-n`, then the same `1..=8` filter |

Getting exposure wrong is invisible until it produces permanent, unclearable diff badges.

### 3.4 Why not ExifTool only

`ExifTool` holds `inner: Mutex<ExifToolInner>` + `child: Mutex<Child>` — one Perl child, all calls
serialized. Per-call latency is not the problem (metadata is a small slice next to an S3 download,
three thumbnail encodes, a blurhash and two hashes); **head-of-line blocking** is: one 4 GB video
parsed under the mutex stalls every other concurrent job's metadata read. That failure mode does not
exist today.

Additionally, every `file_exif` snapshot in the library was produced by rexiv2, so switching the
primary engine re-reads each picture with different value shapes on its next extraction and lights
diff badges library-wide. Going ExifTool-only requires a re-extract-everything backfill.

Prerequisite if ever pursued: a **pool of N stay-open processes** instead of the single `OnceLock` —
not a latency benchmark. Feature 19 §4/§6 to be updated: the startup-latency blocker is resolved by
`-stay_open`; the serialization one replaced it.

## 4. The state machine

Eight values. Every one has a distinct setter, a distinct exit trigger, and a distinct downstream gate.

| State | Direction | Asserts | Edits | Job? |
|---|---|---|---|---|
| `extracting` | read | we have not read this file yet | **refused (409)** | — |
| `extract_failed` | read | we tried and never got an answer | allowed | yes |
| `unsupported_mime` | write | the format cannot receive EXIF writes | DB-only | no |
| `unsupported_file` | read | both engines ran; neither could open it | DB-only | no |
| `pending` | write | DB holds EXIF the file does not; a job exists | allowed (folds) | exists |
| `pending_job_creation` | write | same, and the drain still owes a job | allowed | drain owes |
| `write_failed` | write | the file opened, the write did not land | allowed | re-enqueue |
| `synced` | — | no outstanding sync work | allowed | on edit |

### 4.1 `extracting` (new — and the column loses its default)

Set on the insert paths that are followed by an extraction, and on every re-extraction (WebDAV
overwrite, re-upload, recheck sweep). Left only by a `gen_thumbnail` completion carrying an
extraction outcome — see §6.4 for the one other path that must *not* move it.

`exif_sync_status` **drops its column default**; each insert path states its own:

| Insert path | Status at insert | Why |
|---|---|---|
| Upload (`create`) | `extracting`, or `unsupported_mime` when the MIME is neither EXIF- nor video-readable | an extraction job follows; pre-stamping the unreadable formats avoids a pointless edit-refusal window (§12.3 H) |
| WebDAV PUT (`create`) | same as upload | same |
| Physical copy (`create_copy`) | inherits the source's status and its `file_exif` | identical bytes, so the source's snapshot is valid without reading anything; `is_initial = false` means no extraction will ever report (§12.1 A) |
| Federation receive (`create_received`) | `synced` (inert) | no local original; never reaches the write path |

A copy whose **source** is itself in a non-terminal read state (`extracting`, `extract_failed`)
inherits `extract_failed` rather than a state nothing will resolve — no successful read of those
bytes has happened, edits stay allowed, and a re-extract can settle it.

Replaces the `thumbnails_generated_at IS NULL && extracting_mimes()` guard in both edit paths
(`services::jobs`), closing the open roadmap item under feature 31 ("Extraction-done flag — both edit
paths gate on `thumbnails_generated_at` as a proxy"). It is the **only** state that refuses edits.

`file_exif`: NULL on first ingest, present-but-stale on re-extraction. The promoted EXIF columns may
hold client-supplied values from upload — which is exactly what "we have not read the file" describes.

### 4.2 `extract_failed` (new)

Set by the **job watchdog** when a `gen_thumbnail` exhausts its retry budget — the new arm mirroring
the existing `EditPicture` one — and inherited by a copy whose source had no successful read (§4.1).
An absence of a verdict, not a verdict: no successful read of these bytes has happened and nothing
explains why.

Edits are **allowed and enqueue a job**: the file is probably writable, we simply never read it. An
edit repairs the row naturally — the write job's read-back populates `file_exif` and converges to
`synced`. This is the load-bearing difference from `unsupported_file`.

### 4.3 `unsupported_mime` (renamed from `unsupported`)

Asserts: **this picture's format cannot receive EXIF writes; edits stay DB-only.** A write verdict,
not a read one — the distinction matters because videos are readable (ffprobe) and unwritable, so a
single "supported" axis would be wrong for them.

Two setters, both asserting the same thing:

- **Ingest**, when the MIME is neither EXIF- nor video-readable (GIF, BMP …). Stamped by the backend
  at insert and confirmed by the worker's `UnsupportedMime` outcome at completion — the same
  `supports_exif` call on the same input, so they cannot disagree for a known MIME. An unknown MIME
  (`None`) is `extracting` on both sides: the backend cannot judge it and the worker attempts
  extraction anyway (`unwrap_or(true)`), which resolves it to a real verdict. The **write** side
  defaults the same way — see §10.
- **The edit path**, when an edit lands on a format that is not EXIF-writable. This is where a video
  arrives: it ingests, is read successfully, reaches `synced`, and only becomes `unsupported_mime`
  when someone first tries to write to it — which is today's behaviour, now with a label that says
  what it means.

Exit: the admin recheck sweep after an allowlist change (§8).

### 4.4 `unsupported_file` (new)

Set by a `Failed` outcome — after dispatch **and** fallback on the read path, or from the write
engine on `fail_job`. Both directions report it through the same `ExifExtraction` (§5), so a write
that failed because the *format* takes no EXIF lands `unsupported_mime`, not here. Exit: new bytes,
or an engine-upgrade sweep. Never a retry against the same bytes — the success rate is zero and a
Retry button would be a lie.

### 4.5 Write-direction states

`pending`, `pending_job_creation` and `write_failed` keep their feature-31 semantics unchanged.
`pending_job_creation` is an internal worklist marker and **must render as "pending"** — it is absent
from `SYNC_BADGE`, so the `?? picture.exif_sync_status` fallback currently prints the raw enum to
users after a batch edit.

### 4.6 `synced`

Now means only what it says: no outstanding sync work, established by an observation — extraction
success, convergence against the bound target, or revert-to-file.

A format that is readable but not writable (video) **does** legitimately reach `synced` after ingest:
we read the file and the DB matches it. It moves to `unsupported_mime` only when an edit first tries
to write. A format that is neither readable nor writable never arrives here at all — it is stamped
`unsupported_mime` at insert (§4.1).

### 4.7 Invariants

- `file_exif IS NOT NULL` ⟺ at least one successful extraction or read-back happened. It is **not**
  in bijection with any status — which is why retriability cannot be derived from it.
- `extracting` is the only state that refuses edits.
- `unsupported_mime` and `unsupported_file` are the only states that suppress job creation.
- For received (federated) rows the column is inert: they never reach the write path (`is_owned()`
  guards) and have no local original to extract.

## 5. Worker → backend contract

`CompleteJobRequest.exif: Option<ExtractedExif>` was overloaded three ways — non-initial job,
MIME-skipped, and attempted-but-failed. Replace it with an outcome:

```rust
pub enum ExifExtraction {
    Extracted(ExtractedExif),   // → synced (+ file_exif), or the edit-path read-back
    NotAttempted,               // non-initial gen_thumbnail — status untouched
    UnsupportedMime,            // → unsupported_mime
    Failed,                     // → unsupported_file  (terminal, both engines)
}
```

The same outcome rides on `FailJobRequest`, because a job that dies still knows what it read (§6.5)
— and it is the **write** direction's channel too. `FailJobRequest` carried a parallel
`unsupported: bool` for that, which is the identical overloading one paragraph up, one direction
over: a boolean cannot separate "this format takes no EXIF writes" (`unsupported_mime`) from "these
bytes would not open" (`unsupported_file`), so every write failure was recorded as the latter. The
bool is gone; both directions report the same four variants, and the backend has one mapping from
outcome to status (`terminal_verdict`) with a per-caller fallback.

Retriable failures have no representation on the *complete* path: they never reach `complete_job`.
On the fail path they arrive as `NotAttempted` with `permanent = false`, which the backend ignores —
the job will be retried. On the edit path `NotAttempted` on a *permanent* failure means the file
opened and the write did not land: `write_failed`.

The write path reaches `UnsupportedMime` from a preflight, before the download — the verdict is
about the MIME, so fetching the bytes is waste, and on a video that is the whole container.

## 6. Flows

### 6.1 Ingest

1. Row inserted → `extracting`.
2. `gen_thumbnail` (`is_initial`) downloads, then extracts **before** thumbnails/hash/upload, so a
   retriable failure costs one download and nothing else.
3. Outcome:
   - `Extracted` → `update_from_worker` (promoted columns + `exif_data` camera keys + `file_exif`) → `synced`
   - `UnsupportedMime` → `unsupported_mime`
   - `Failed` → `unsupported_file`
   - retriable → **fail the job**; the existing retry budget and watchdog apply. Budget exhausted →
     watchdog sets `extract_failed`.

### 6.2 Re-extraction (overwrite, re-upload, sweep)

Sets `extracting`, then §6.1 from step 2. Per 31 §5 an external overwrite is a new source of truth
and resets Archypix-side edits that had not reached the file.

### 6.3 A successful extraction clears a stale verdict

`update_from_worker` currently preserves `unsupported` against re-extraction. It is only ever reached
when extraction **succeeded**, which is direct evidence against the verdict, and the guard would now
also strand a file the user replaced with a good one. Remove the `CASE`: a successful extraction sets
`synced`. Extraction only runs on new bytes, so re-evaluating the verdict there is correct.

### 6.4 An edit completion never leaves `extracting`

`complete_job`'s convergence branch (31 §3.3) must skip the status write when the row is
`extracting`. Without the guard, a WebDAV overwrite that lands while an `edit_picture` job is in
flight sets `extracting`, and the edit's completion then writes `synced`/`pending_job_creation` over
it — losing the invariant and, worse, overwriting the fresh `file_exif` with the stale job's
read-back of the pre-overwrite bytes.

The stale job's `file_exif` write is skipped for the same reason and by the same condition. This
narrows (does not close) the known 31 §5 gap: the claimed job still writes its older target onto the
new bytes, but it no longer corrupts the recorded file state, and the in-flight extraction settles
the row.

### 6.5 A permanently failed extraction job

`complete_job` is not the only exit. A `gen_thumbnail` that fails **permanently** — a codec error in
the thumbnailer, a dead upload, a missing presigned URL — never completes, and the watchdog only
rescues *budget exhaustion*. Without a fail-path rule the picture stays `extracting` forever with no
job left to move it, and §4.1 refuses every edit on it: the same stranded-state bug this feature
exists to remove, one layer down.

So the worker publishes its extraction outcome as soon as it has one, and reports it on
`fail_job` too. For a permanently failed `is_initial` job the backend settles the row:

| Outcome | Row becomes |
|---|---|
| `Extracted` | `synced`, with the EXIF and `file_exif` recorded — the read succeeded even though the job did not |
| `UnsupportedMime` | `unsupported_mime` |
| `Failed` | `unsupported_file` |
| `NotAttempted` | `extract_failed` — the job died before reaching a verdict |

Two constraints:

- **Never stamp `thumbnails_generated_at`** when recording an `Extracted` outcome here. The
  thumbnails are exactly what failed; stamping it would serve a missing thumbnail
  (`services::pictures` falls back on the null check) and hide the row from the
  `regenerate-thumbnails` "missing" scope. `update_from_worker` takes a `set_thumbnails` flag for
  this, mirroring `update_after_processing`.
- **Only when the failure is permanent, and only for `is_initial`.** A retriable failure is not a
  verdict — the row stays `extracting` and the retry budget owns it. A `reextract_exif = false`
  thumbnail regeneration says nothing about the read direction.

Every write is conditional on the row still being `extracting`, so a re-extraction that landed while
this job was dying keeps its newer verdict.

## 7. Failure-classification fixes

Without these the retriable branch of §6.1 is unreachable:

- `imaging/video.rs` — a missing/unspawnable `ffprobe` maps to `WorkerError::Exif` (permanent). It is
  a worker-environment fault: `ToolUnavailable`.
- `imaging/exif.rs` — `Metadata::new_from_path` failure is classified `UnsupportedFormat`
  unconditionally, folding a truncated download or an IO error into a format verdict. Distinguish the
  IO kind before classifying.
- `imaging/exif.rs` — `static FORCED` in `exiftool()` is declared and never read. Remove.

## 8. Re-extract and the recheck sweep

**Per picture** — `POST /api/authenticated/pictures/{id}/exif/reextract` enqueues a `gen_thumbnail`
with `is_initial = true` and sets `extracting`. Distinct from `resync`, which enqueues an *edit* job
in the opposite direction and accepts only `pending`/`write_failed`.

It passes **no idempotency key** and guards on "no pending/processing `gen_thumbnail` for this
picture" instead, mirroring `enqueue_if_absent_edit`. The existing key
`gen_thumbnail_initial:{picture_id}` is permanent and globally unique, so reusing it makes every
re-extraction fail with a 409 until job cleanup prunes the original row (§12.1 B). `regenerate_thumbnails`
already opts out of the key for exactly this reason. The sweep uses
`gen_thumbnail_reextract:{picture_id}:{sweep_id}` — idempotent within one sweep, never against the
previous one.

Refused (409) when the row is `pending`, `pending_job_creation` or `write_failed`: extraction makes
the file authoritative and would silently discard an unsynced DB edit.

**Bulk** — an admin sweep, implemented as a **routine** (03 §H) with a bounded batch per tick,
triggered by `POST /api/admin/pictures/recheck-exif`. The SQL is trivial; the hazard is enqueue rate —
a matching sweep can be the whole library, and flipping a million rows while enqueuing a million jobs
is an incident, not a migration. The routine gives backpressure and resumability for free.

`scope`:

- `mime` — rows in `unsupported_mime`, optionally filtered to the MIMEs that just became supported
  (the normal case after an allowlist bump).
- `file` — rows in `unsupported_file`, after an engine upgrade. Rare and mostly futile; never the default.
- `failed` — rows in `extract_failed`, after a tool outage.

The sweep skips `pending`/`pending_job_creation`/`write_failed` for the reason above.

`POST /api/admin/pictures/regenerate-thumbnails` with `reextract_exif = true` is a re-extraction
trigger too and must set `extracting` on the rows it touches, or an edit can race the in-flight
extraction — the race 04 §11.2 exists to prevent. With `reextract_exif = false` it stays as it is
(`NotAttempted`, status untouched).

## 9. Schema & migration

Postgres 18, so `RENAME VALUE` (PG 10+) and in-transaction `ADD VALUE` (PG 12+) are both available.
`ADD VALUE` is never *used* in the migration that adds it — the 31 §2.1 pattern.

```sql
ALTER TYPE public.picture_exif_sync_status RENAME VALUE 'unsupported' TO 'unsupported_mime';
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'unsupported_file';
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'extracting';
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'extract_failed';
ALTER TABLE pictures ALTER COLUMN exif_sync_status DROP DEFAULT;
```

The rename is chosen over add-and-migrate because most existing `unsupported` rows *are* the MIME
verdict (the preflight is the common setter). The default is **dropped**, not repointed: a default
would assume an extraction follows, which three of the four insert paths do not do (§4.1).

Follow-up migration (separate file, so the new label is committed):

- Promote genuine file verdicts: rows `unsupported_mime` whose MIME **is** in the allowlist →
  `unsupported_file`. The allowlist is inlined here as a one-time historical snapshot — the correct
  place for it, and the last time it appears in SQL anywhere.

`RENAME VALUE` is safe here on the explicit constraint that **no old backend binary runs against the
migrated DB** — the Rust enum maps to the label by name, so an instance on the previous binary would
fail to decode every row carrying it. Migrate during the deploy, not alongside a live old instance.

**Down migration.** Postgres cannot drop an enum value (see the 0013 down), so the down folds the new
labels back and leaves them orphaned:

```sql
UPDATE pictures SET exif_sync_status = 'synced'
 WHERE exif_sync_status IN ('extracting', 'extract_failed');
UPDATE pictures SET exif_sync_status = 'unsupported_mime' WHERE exif_sync_status = 'unsupported_file';
ALTER TYPE public.picture_exif_sync_status RENAME VALUE 'unsupported_mime' TO 'unsupported';
ALTER TABLE pictures ALTER COLUMN exif_sync_status SET DEFAULT 'synced';
```

Index `idx_pictures_exif_pending` stays; add partial indexes for the sweep worklists if the routine's
scan shows up.

## 10. What this deletes

- `services::jobs::supported_mimes()` and `extracting_mimes()`, and the two `Vec<String>` parameters
  threaded through `count_owned_unsupported_selection` and both `batch_apply_exif_owned_selection`
  calls — the supported/unsupported partition becomes a status predicate instead of the same query
  run twice with a boolean flag.
- The `thumbnails_generated_at IS NULL` extraction proxy in both edit paths.
- The unknown-MIME `unwrap_or(true)` / `unwrap_or(false)` asymmetry.
- The `CASE … 'unsupported'` guard in `update_from_worker`.
- `static FORCED`.

**One derivation is deliberately kept**: the per-picture edit path re-checks `supports_exif(mime)` on
the already-loaded row. It is free there and closes the window between an allowlist change and the
sweep running, where an edit would otherwise be silently DB-only with no job. Set-based batch,
aggregates and UI use the stored state.

**An unknown MIME (`None`) is treated as capable everywhere** — `ingest_exif_status`, the worker's
extraction branch, and that kept derivation all default to "attempt it". Absence of a MIME is a gap
in our own metadata, not evidence about the file, and `unsupported_mime` is terminal: stamping it
from missing information would suppress every future job on a guess, which is the assert-what-we-
never-observed bug this feature exists to remove. The write is attempted and the worker returns a
real verdict (`unsupported_file` if no engine can open it). The cost is one doomed job, once.

## 11. API & frontend

- `ExifSyncStatus` gains `extracting`, `extract_failed`, `unsupported_file`; `unsupported` →
  `unsupported_mime` (06 §10 shared-type reference, and every DTO carrying the field).
- `POST /pictures/{id}/exif/reextract`; `POST /api/admin/pictures/recheck-exif`.
- `SYNC_BADGE`: `extracting` → "reading metadata"; `extract_failed` → "metadata unread" + Re-extract;
  `unsupported_mime` → "n/a"; `unsupported_file` → "unreadable"; `pending_job_creation` → "pending".
  No raw-enum fallthrough.
- The aggregate's `unsupported` bucket splits in two. Today it under-reports — a GIF only becomes
  `unsupported` once someone edits it — so the counts change meaning and become correct.
- Received rows render no sync badge at all.

## 12. Edge cases

All found in the post-spec verification pass; all resolved in the sections referenced.

| # | Case                                                                                                                          | Resolution                                                                                    |
|---|-------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------|
| A | Physical copies would sit in `extracting` forever (`is_initial = false` reports `NotAttempted`), refusing every edit with 409 | No column default; copies inherit the source's status and `file_exif` — §4.1                  |
| B | `POST /exif/reextract` and the sweep collide with the permanent, globally-unique key `gen_thumbnail_initial:{pid}`            | No key + an in-flight guard; sweep-scoped key — §8. The key itself was fixed later, see §12.2 |
| C | `RENAME VALUE` breaks an old binary reading the renamed label                                                                 | Accepted: no old backend runs against the migrated DB — §9                                    |
| D | An `edit_picture` completion overwrites `extracting` and clobbers a fresh `file_exif` with a stale read-back                  | The convergence branch skips both writes while `extracting` — §6.4                            |
| E | `regenerate_thumbnails(reextract_exif = true)` enqueues an extraction without setting `extracting`, letting an edit race it   | The admin path sets `extracting` too — §8                                                     |
| F | Received rows would inherit `extracting` and never leave it                                                                   | `create_received` sets `synced` explicitly — §4.1                                             |
| G | The down migration cannot drop the added enum values                                                                          | Folds them back and leaves the labels orphaned — §9                                           |
| H | A format that is neither EXIF- nor video-readable would refuse edits during its ingest window                                 | Stamped `unsupported_mime` at insert — §4.1, §4.3                                             |

### 12.1 Still open, inherited from feature 31

A reconcile already claimed when an external overwrite lands still writes its older target onto the
new bytes (31 §5). §6.4 stops it corrupting `file_exif`, and the in-flight extraction settles the
row, but the file briefly carries the pre-overwrite target. Unchanged by this feature.

### 12.2 Noted, out of scope — since fixed

The job idempotency mechanism was not idempotent: `JobRepository::create` had no `ON CONFLICT`, so a
duplicate key surfaced as a 23505 → `AppError::Conflict` rather than returning the existing job; the
key was never scoped to liveness, so it stayed burned until `JobCleanupRoutine` pruned the row; and
`jobs` carried two overlapping unique constraints (`idempotency_key UNIQUE` and
`uq_job_idempotency (owner_id, idempotency_key)`), the global one subsuming the composite and forcing
owner scoping to be encoded in the key string. This feature avoided it (§8) rather than fixing it.

Fixed afterwards by migration `0017_job_idempotency_liveness`: both constraints are replaced by the
partial unique index `uq_jobs_idempotency_live (owner_id, idempotency_key) WHERE idempotency_key IS
NOT NULL AND status IN ('pending','processing')`, and `JobRepository::create_idempotent` upserts
against it, returning the live job instead of a conflict. §8's two workarounds stand on their own
merits and were kept: a re-extract has no content to key on, and an explicit user action deserves an
explicit `409` over a silent dedupe.

## 13. Testing

- `worker/src/imaging/exif.rs` — differential parity across a corpus: mapped `FullExif` equality per
  §3.3, including a file with a rational exposure and one with below-sea-level altitude.
- Dispatch: a BMFF file reads via ExifTool without touching rexiv2; a corrupt JPEG falls back; a
  file neither engine opens yields `Failed`; `ToolUnavailable` during fallback stays retriable.
- `worker_contract.rs` — each outcome lands its state; a retriable extraction failure fails the job
  and does not touch the status; watchdog exhaustion sets `extract_failed`; a successful
  re-extraction clears `unsupported_file`.
- `services_jobs.rs` — `reextract` refuses `pending`/`write_failed`; accepts `extract_failed`.
- `batch_editing.rs` — the batch partition by status matches the old MIME partition on a mixed set.
- Migration: an `unsupported` row with an EXIF-capable MIME promotes to `unsupported_file`; one
  without stays `unsupported_mime`.

## 14. Documentation updates

- `04_WORKER_ARCHITECTURE.md` — Error policy (L91) and the EXIF sections: dispatch + fallback, the
  outcome enum, extraction failure policy.
- `31_robust_exif_sync.md` — §3.1 (extraction sets `extracting` → observed state), §6 (unsupported
  split in two), §8 status.
- `19_exiftool_metadata_engine.md` — §4/§6: the startup-latency blocker is resolved; mutex
  serialization replaced it; a process pool is the new prerequisite.
- `06_API_REFERENCE.md` — §10 enum, the two new endpoints.
- `99_ROADMAP_MVP.md` — new entry; close the "Extraction-done flag" sub-item under feature 31.

## 15. Work breakdown

- [x] Worker: `exiftool_read` + MIME dispatch + failure fallback (§3.1–3.2).
- [x] Worker: differential parity test + the four normalizations (§3.3).
- [x] Worker: classification fixes — ffprobe `ToolUnavailable`, IO vs format on open, drop `FORCED` (§7).
- [x] Common: `ExifExtraction` outcome on the job response (§5).
- [x] Worker: `thumbnail.rs` — three branches instead of one `warn!`; fail the job on retriable (§6.1).
- [x] Migration: rename + three values + **drop** the column default; down migration; follow-up
      split migration (§9).
- [x] Backend: `complete_job` outcome handling; drop the `update_from_worker` CASE (§6.3).
- [x] Backend: watchdog arm for `gen_thumbnail` → `extract_failed` (§4.2).
- [x] Backend: edit paths gate on `extracting`; delete both MIME helpers and the threaded params (§10).
- [x] Backend: per-insert-path status (upload / WebDAV / copy-inherits / received), §4.1.
- [x] Backend: `complete_job` edit branch skips status + `file_exif` while `extracting` (§6.4).
- [x] Backend: `reextract` endpoint (no idempotency key + in-flight guard) + admin recheck routine &
      endpoint; `regenerate_thumbnails(reextract_exif)` sets `extracting` (§8).
- [x] Frontend: badges, Re-extract action, aggregate split (§11).
- [x] Tests (§13); docs (§14).

## 16. Implementation status

Implemented. Migrations `0015_exif_read_path` + `0016_split_unsupported_exif`; routine
`exif_recheck`; setting `exif_recheck_batch`.

Deviations and residue, all deliberate:

- **§9's SQL block said `SET DEFAULT 'extracting'`, which contradicts §4.1, §12 A and §15** ("drops
  its column default"). The three that agree won: the column has **no** default and every insert path
  states its own status. The down migration restores `DEFAULT 'synced'`, which only makes sense
  against a dropped default.
- **ExifTool refuses very little.** It reports "no metadata" far more often than an error (garbage
  bytes under a `.jpg`, `.heic` or unknown extension all come back as an empty result), so
  `unsupported_file` is reachable almost exclusively through the *write* path's `unsupported: true`
  and through an ExifTool process error. §8's `scope: file` sweep is correspondingly rare — as the
  spec already expected.
- **Parity is asserted with a numeric tolerance on GPS degrees**, not bit equality: both engines
  reassemble the coordinate from the stored deg/min/sec rationals and the last ULPs differ
  (`48.858222` vs `48.858222000000005`). Every other mapped field is compared exactly. The tolerance
  is the one the diff badges already use (31 §8), so no badge can light from the difference.
- **`ExposureTime` above ~1/4 s loses the stored denominator.** ExifTool's print form is `%.1f` there
  (a stored `5/4` prints as `1.2`), so the parse yields `6/5`. Exactly-representable decimals
  (`0.5 → 1/2`, `2 → 2/1`, `30 → 30/1`) round-trip, and the fraction form — every exposure below
  1/4 s — is exact. Reading `-n` instead would mean rationalizing a float, which §3.3 forbids.
- **A batch EXIF edit over a video was mislabelled — fixed in §5, no state-machine change.** A batch
  partitions on stored status (§10), so a video — legitimately `synced` (§4.6) — enqueued a write
  job, and the failure landed `unsupported_file` ("unreadable") instead of `unsupported_mime`
  ("n/a").

  §4.3 already described the intended behaviour: a video reaches `synced` and takes the write verdict
  when something first tries to write it. That holds for the per-picture path, which derives it
  locally. The batch path goes through the worker, and the verdict could not survive the trip:
  `FailJobRequest` flattened it to `unsupported: bool`, which cannot separate a format verdict from a
  file verdict — the same overloading §5 had already removed from the read direction, left standing
  on the write one.

  So the fix was a deletion, not an amendment: drop the bool, report both directions through
  `ExifExtraction`, and collapse the backend's three near-identical mappings into one
  (`terminal_verdict` + a per-caller fallback). The worker reaches `UnsupportedMime` from a preflight
  *before* the download, so the doomed fetch never happens either.

  An earlier attempt amended §4.6 instead, settling videos on `unsupported_mime` at extraction
  success with a backfill migration. It worked, but it added a helper, a repository parameter, three
  call sites and a migration to fix what was a lossy channel, and it made the spec describe the
  workaround rather than the intent. Reverted in favour of the above.

  Still open, and genuinely a §10 consequence rather than a channel bug: a batch **dry run** previews
  a video under `edited`, and the aggregate counts it under `synced`, because both read the stored
  status and the verdict only exists after a write is attempted. Tracked on the roadmap.
- **A copy of a `pending` source can sit in `pending` with no job.** §4.1's inheritance rule carves
  out only the two never-read states; a copy of a row with an unsynced edit inherits `pending` while
  `is_initial = false` enqueues no reconcile. Recoverable through `/exif/resync`, and visible in the
  admin "stuck pending" health query. Tracked on the roadmap.
- **`fail_job` settles a permanently failed extraction** (§6.5), added after the first pass: without
  it a `gen_thumbnail` that extracted fine and then hit a codec error left the picture stranded in
  `extracting` with its EXIF thrown away.
- **The §13 migration test is not implemented.** `sqlx::test` applies every migration before the test
  body runs, so there is no seam to seed a pre-0016 `unsupported` row from. The equivalent invariant
  is covered behaviourally instead (the batch partition, and the two terminal states' distinct
  handling).
