# Feature 31: Robust EXIF State-Based Synchronization

## 1. Overview & Goals

Feature 14 introduced EXIF batch edits with a "deferred command" model: the DB was updated
immediately and a background routine (`exif_drain`) later created a reconcile job carrying a delta to
apply to the file. That model had three flaws:

1. **State loss** — a batch update overwrote the previous values before the reconcile job existed,
   losing the revert baseline.
2. **Fragile revert** — the automatic revert on failure trusted the job config's `previous`, which
   was empty or wrong for batch edits.
3. **Complex concurrency** — successive edits needed delta "folding" plus follow-up jobs.

Feature 31 replaces the command model with a **state model**:

- The DB records the last EXIF state known to be *in the file* (`pictures.file_exif`).
- Syncing means bringing the file from `file_exif` to the DB's `full_exif()`.
- A permanent write failure is surfaced (`write_failed`) instead of silently reverted; the user
  retries, edits again, or reverts the DB to the file.

## 2. Structural Changes

### 2.1 Database schema

New migration (`0013_robust_exif_sync`, per 03 §I — never edit an applied migration):

```sql
ALTER TABLE pictures ADD COLUMN IF NOT EXISTS file_exif JSONB;  -- NULL for received pictures
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'write_failed';
```

The migration **backfills** `file_exif` from the promoted columns + `exif_data` for owned, extracted
rows already marked `synced`, so revert-to-file works on the existing library without waiting for a
re-extraction. `ADD VALUE` is never *used* in the same migration, so it is transaction-safe.

### 2.2 Domain model (`back/src/domain/picture.rs`)

```rust
pub enum ExifSyncStatus { Synced, Pending, Unsupported, PendingJobCreation, WriteFailed }

pub struct Picture {
    pub file_exif: Option<Json<FullExif>>, // the file's own snapshot; None until first extraction
}
```

## 3. The State-Based Sync Workflow

### 3.1 Ingestion & extraction

A `gen_thumbnail` job with `is_initial` returns the extracted EXIF. `update_from_worker` writes it to
the promoted columns **and** `exif_data`'s camera keys **and** `file_exif`, and sets `synced` — unless
the row is `unsupported`, a terminal state extraction must not undo.

### 3.2 Editing (single or batch)

1. The backend updates the `pictures` row (promoted columns + `exif_data`).
2. **`file_exif` is left untouched** — it still describes the file.
3. Status becomes `pending` (single) or `pending_job_creation` (batch; the drain creates the job).
4. The work outstanding is the difference between `full_exif()` and `file_exif`.

An edit is refused (409) while the initial extraction is still running, since the extraction would
overwrite it — the set-based batch path applies the same rule by skipping those rows (04 §11.2).

### 3.3 Reconcile job lifecycle

`ExifEdit` carries a single `target: FullExif`, and the enqueuing side stores a placeholder.

**At claim.** The backend rebinds `target` to the picture's current `full_exif()`, hands that to the
worker, **and persists it on the job row**. Five edits between enqueue and claim collapse into one
write of the latest state, and the completion handler knows exactly which state the file reached.

**At completion.**

1. The worker rewrote the file to `target`, read the file back, and returned that read-back.
2. The backend stores the read-back in `file_exif`.
3. It compares the **DB row** against the **job's target**:
   - equal → `synced`;
   - different (an edit landed mid-flight) → `pending_job_creation`, and the drain enqueues the
     follow-up.

The read-back is deliberately *not* the convergence criterion: EXIF stores GPS and exposure as
rationals, so a round-trip never returns the exact `f64` it was given, and an equality test against
it would leave every picture permanently `pending`. `file_exif` is what the file holds (for revert
and for the diff badges); the target is what the sync intended.

**On permanent failure.** No auto-revert: the DB keeps the user's edit, the file keeps `file_exif`,
and the status becomes `write_failed`. A reconcile whose retry budget is exhausted by the **job
watchdog** never reaches `fail_job`, so the watchdog marks those pictures itself.

### 3.4 Write semantics (worker)

`write_exif_target` makes the file match the target exactly: every `Some` field is written, every
absent one deleted. Deletion is **grouped**: GPS (lat/lng/alt) and exposure (num/den) share one tag
group in both writers, so they are only cleared when every member is absent — a target with
coordinates but no altitude writes the coordinates and drops just the altitude tag.

Consequence, by design: a field the extractor cannot represent is dropped from the file on the first
sync, because the DB is the source of truth for every editable field. Only tags the extractor reads
are ever cleared, so this is limited to rows whose EXIF was seeded from somewhere other than the file
(e.g. a physical copy, feature 11).

## 4. User Actions on Failure

A `write_failed` picture offers:

1. **Retry** — `POST /pictures/{id}/exif/resync` re-enqueues the reconcile (target rebound at claim).
2. **Edit** — a new edit enqueues a job as usual.
3. **Revert to file** — `POST /pictures/{id}/exif/revert` rewrites the row's EXIF columns from
   `file_exif` and marks it `synced`. Refused (409) with no snapshot, or while a job is in flight —
   that job would write its pre-revert target back and re-diverge the row. The panel disables the
   action outright when `file_exif` is null, since it could only 409.
4. **Revert one field** — draft-only, in the field's popup: the field takes its `file_exif` value (nothing, when the picture has no snapshot) and Save
   writes it through as an ordinary edit. This
   cannot clear `write_failed` on its own — only a successful write or (3) does.

## 5. External Overwrites (WebDAV)

A file overwritten through WebDAV (or re-uploaded) enqueues an extraction (`is_initial = true`). The
backend then updates **both** `file_exif` and the picture's metadata columns from the new file and
sets `synced`: an external overwrite is a new source of truth and resets Archypix-side edits that had
not yet reached the file.

*Known gap:* a reconcile already claimed when the overwrite lands will still write its (older) target
onto the new file. Rare, and self-correcting on the next extraction.

## 6. Refined Unsupported Handling

- **MIME preflight** still gates job creation for formats that cannot embed EXIF.
- **Worker-detected** — only a file the metadata library cannot *open at all* yields
  `WorkerError::UnsupportedFormat`, which sets `unsupported: true` in the fail body. A write that
  fails on a file that opened fine is `WorkerError::Exif` → `write_failed`; a read-back failure
  after a successful write is downgraded to `write_failed` for the same reason.
- **Tool unavailable** — a missing or unspawnable `exiftool` is a worker-environment fault, not a
  file verdict: `WorkerError::ToolUnavailable` is *retriable*, so the picture stays `pending` and
  only reaches `write_failed` via the watchdog once the retry budget is spent.
- **Terminal state** — the backend sets `unsupported` rather than `write_failed`; retrying can never
  help. Revert-to-file still works, and a later edit on such a row stays DB-only (no job is
  enqueued) — `unsupported` is never flipped back to `pending` by the edit path.

## 7. Batch Edit & Drain Robustness

`ExifDrainRoutine` finds `pending_job_creation` rows, creates one reconcile job each, and flips them
to `pending`. It carries no delta, so any number of batch edits collapse into a single job whose
target is bound at claim-time. Because `file_exif` is preserved throughout, the "before" state is
never lost.

## 8. Implementation Status

- [x] Migration: `file_exif` + `write_failed`, with a backfill for already-synced rows.
- [x] `PictureRepository`: `file_exif` in every read, `update_from_worker` (extraction),
      `set_file_exif` (read-back); revert reuses `write_exif_snapshot`.
- [x] Claim-time target binding, persisted on the job row.
- [x] Completion convergence against the bound target; mid-flight edits return to the drain.
- [x] `fail_job` → `write_failed` / `unsupported`; watchdog marks retry-exhausted reconciles.
- [x] Narrow `unsupported` to open failures; retriable `ToolUnavailable`; edit path skips job
  creation for `unsupported` rows.
- [x] Batch path skips still-extracting rows (04 §11.2 parity).
- [x] `POST /api/authenticated/pictures/{id}/exif/revert`.
- [x] Worker: `write_exif_target` with grouped clears + read-back.
- [x] Frontend: `write error` badge, Retry / Revert actions, per-field `diff` badges (numeric
      tolerance), `write_failed` count in the batch panel.
- [x] Tests: `worker_contract.rs` (binding, convergence, failure states, extraction overwrite),
      `services_jobs.rs` (revert + resync), `batch_editing.rs` (extraction guard),
      `worker/src/imaging/exif.rs` (clear grouping).
