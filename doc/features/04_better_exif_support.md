# Better EXIF Support

## 1. Overview & goals

Roadmap item **"Exif edition"** ([99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md)), expanded to:

1. **Edit and batch-edit EXIF/metadata** for owned pictures.
2. **Guarantee the S3 original's embedded EXIF converges to the DB** — a WebDAV
   requirement. The DB is the source of truth and is updated synchronously; the file is
   rewritten by a worker job within seconds/minutes. A file left out of sync is a tracked
   inconsistency, never a steady state.
3. **Add EXIF & geo data to picture announcements** so federated recipients receive the
   same metadata the owner has (today only `captured_at` crosses the wire), and **propagate
   subsequent edits** to recipients.

Scope is **EXIF/metadata only**. Visual pixel edits (crop/resize/brightness) remain a
v1.0 item; this spec keeps the `visual` branch of `edit_picture` working and wires the
versioning policy for it, but does not implement it.

EXIF **edit history is out of scope** (v1.0 roadmap item). EXIF-only edits keep no
history; do not lean on the `jobs` table as a history store (it is pruned by the job
cleanup task — see [03_recurring_tasks_framework.md](03_recurring_tasks_framework.md)).

## 2. Decisions (settled)

- **Write-through + guaranteed convergence (§4).** Synchronous DB write, asynchronous
  mandatory file reconcile. No DB-only mode, no `persist_to_file` flag.
- **Revert on permanent failure (§4.3).** The reconcile job carries the picture's previous
  and new EXIF; if it permanently fails, the DB row is rolled back so DB and file end
  consistent at the *old* state. No revision counter, no forward-heal sweep.
- **One in-flight reconcile per picture (§5)** with fold-while-pending and a
  completion-time re-enqueue when the DB moved on. Correct under rapid successive edits and
  multi-failure.
- **`exif_sync_status` ∈ {`synced`, `pending`, `unsupported`} (§6).**
- **No batch cap** for now (§7.2).
- **MIME preflight at edit time (§8).** Formats that cannot embed EXIF → `unsupported`,
  DB-only, no job. This is the only acceptable terminal divergence.
- **Field clear semantics (§7.3).** `set` / `clear` distinguish unchanged vs set vs
  cleared.
- **Versioning policy (§9):** `None` → never; `OriginalCopy` → keep only the original
  (snapshot on first edit); `FullVersioning` → original + one snapshot per *visual* edit;
  exif-only edits never add a version.
- **Federation (§10):** extend the announced picture with gps/orientation/exif_data;
  re-announce already-tracked pictures whose metadata changed, gated by `updated_at`.

## 3. Current state & bugs to fix

A single-picture edit path exists end-to-end; this feature widens and corrects it.

| Concern            | Where                                                                            | Status                  |
|--------------------|----------------------------------------------------------------------------------|-------------------------|
| Edit endpoint      | `POST /pictures/{id}/edit` (`api/user/jobs.rs::enqueue_edit`)                    | Single, owned-only      |
| Job config         | `EditPictureConfig { picture_id, exif_overrides, visual }` (`common/src/job.rs`) | Evolve (§7.4)           |
| File write         | `worker/src/imaging/exif.rs::write_exif_overrides`                               | Buggy (below)           |
| DB apply           | `PictureRepository::apply_exif_overrides`                                        | Keep; add revert (§7.4) |
| Versioning on edit | `api/worker/handlers.rs` claim path (~L83–116)                                   | Wrong (§9)              |

**Bugs that must be fixed as part of this work:**

1. **Pipeline never woken on edit.** Neither `enqueue_edit` nor the worker-completion
   handler marks the picture dirty or wakes the pipeline, so `gps_within_bbox`,
   `capture_year`/`capture_month` rules and segments never re-evaluate after an edit
   (spec §3.1 `metadata` label). The edit must reset `last_pipeline_run_at = NULL` and call
   `pipeline_waker.wake(user_id)`.
2. **`write_exif_overrides` drops fields.** `focal_length_mm`, `f_number`,
   `exposure_time_num/den` are merged into the DB by `apply_exif_overrides` but never
   written to the file → DB↔file divergence. Cover every editable field, or drop the field.
3. **Worker uploads the original before regenerating thumbnails** (`edit_picture.rs`). The
   revert model requires the modified-original upload to be the **last** fallible step
   (§4.3). Reorder: regen + upload thumbnails from the local edited file *first*, then
   upload the original, then complete.
4. **`OriginalCopy` snapshots on every edit** (treated like `FullVersioning`) — see §9.

## 4. Write-through model

### 4.1 Phase 1 — DB write (synchronous)

In the request transaction: validate ownership; **MIME preflight** (§8); capture the
picture's `previous` editable-EXIF values; apply `set`/`clear` to the row; bump
`pictures.updated_at`; set `exif_sync_status = pending`; reset `last_pipeline_run_at =
NULL`. Enqueue the reconcile job (subject to §5) in the **same transaction**. Commit, then
wake the pipeline. Returns immediately.

If the format cannot embed EXIF: do the DB write but set `exif_sync_status = unsupported`
and enqueue **no** job (§8).

### 4.2 Phase 2 — file reconcile (asynchronous)

An `edit_picture` job rewrites the embedded EXIF in the S3 original to match the row,
recomputes `file_hash` (the WebDAV ETag), and on success sets `exif_sync_status = synced`.

### 4.3 Failure handling — revert

The job config carries both `previous` and the `set`/`clear` delta. On **permanent**
failure (retries exhausted or `permanent: true`):

- **Value-gated revert:** if the row still equals this job's resulting (`new`) values,
  restore `previous` (including re-adding fields that were cleared / clearing fields that
  were added); else do nothing — a newer edit owns the state (§5).
- After reverting, **re-dirty + wake the pipeline** (a revert is itself a metadata change)
  and record the rollback in the job's `error_message` so the UI can tell the user the
  edit did not stick. `exif_sync_status` returns to `synced` (consistent at old values).

**File-untouched-on-failure invariant.** Revert is only correct if a permanent failure
implies the S3 original was not overwritten — hence bug #3's reorder (upload original
last). A crash *after* upload but *before* completion leaves the job `processing` → the
job watchdog resets it → idempotent retry completes forward.

## 5. Concurrency: one in-flight reconcile per picture

Enforced by a **partial unique index** (a static `idempotency_key` cannot be used — keys
are globally `UNIQUE` and rows linger until cleanup):

```sql
CREATE UNIQUE INDEX uq_edit_picture_inflight
    ON jobs (picture_id) WHERE job_type = 'edit_picture' AND status IN ('pending', 'processing');
```

Edit-time behaviour:

- **No in-flight job** → insert one with `previous` = current DB (pre-edit) and the delta.
- **A `pending` (unclaimed) job exists** → **fold**: update that job's `new`/delta to the
  cumulative latest; leave its `previous` (the file is still at the synced baseline).
- **A `processing` job exists** → apply the DB edit and set `pending`, but **do not
  enqueue** (the unique index would reject it anyway). The completion handler re-enqueues.

Completion handler (success *or* revert), after applying its effect, computes the file's
**actual** current state (`= new` on success, `= previous` on a value-matched revert, or
the untouched baseline if the revert was suppressed) and, if the DB now differs from it,
**enqueues a follow-up job** with `previous` = that actual file state and the new delta.
This is what makes chained edits correct even across failures — every job's `previous`
reflects the file's true content, never an assumption.

> Stuck-`pending` safety net (optional): a low-frequency recurring task (using the
> framework in [03](03_recurring_tasks_framework.md)) that re-enqueues pictures left in
> `pending` with no in-flight job — covers a crash inside the completion handler before the
> follow-up enqueue. Not required for correctness in the happy path; nice-to-have.

## 6. Schema changes

Edit `back/migrations/001_initial_schema.up.sql` directly (single-migration policy), then
`cd back && cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare`.

1. **`pictures.exif_sync_status`** — new enum, default `synced`:
   ```sql
   CREATE TYPE picture_exif_sync_status AS ENUM ('synced', 'pending', 'unsupported');
   ALTER TABLE pictures ADD COLUMN exif_sync_status picture_exif_sync_status NOT NULL DEFAULT 'synced';
   ```
   (Apply inline in the table definition, not as a separate `ALTER`, to match the file's
   style.) Optional partial index `WHERE exif_sync_status = 'pending'` for the safety-net
   sweep.
2. **`jobs`** — the partial unique index from §5.
3. **`share_announcements.announced_updated_at TIMESTAMP`** — the picture `updated_at` value
   captured at last successful (re-)announce; gates metadata re-announce (§10.3).

`pictures.exif_data` (JSONB, GIN-indexed) and the promoted columns (`captured_at`,
`gps_lat/lng/alt`, `orientation`) are unchanged structurally.

## 7. Domain & API

### 7.1 Single-picture edit (revised)

`POST /api/authenticated/pictures/{id}/edit` — request body becomes the `set`/`clear`
shape (§7.3). Handler does Phase 1 (§4.1). Response: the updated picture row, its
`exif_sync_status`, and the `job_id` (or `null` when `unsupported`).

### 7.2 Batch edit (new)

`PATCH /api/authenticated/pictures/exif` — mirrors `PATCH /api/authenticated/tags`.

```jsonc
{
  "picture_ids": ["…", "…"],     // owned pictures only; no cap for now
  "set":   { "captured_at": "2024-08-03T10:15:00", "gps_lat": 45.92, "gps_lng": 6.87 },
  "clear": ["gps_alt", "orientation"]
}
```

- Validate **all** ids belong to the caller and are owned (not received) before any
  mutation; reject the whole batch on first violation (return the offending id).
- Apply `set`/`clear` to all rows in one transaction (chunk with `unnest`-style array
  binding), capturing each row's `previous`, bumping `updated_at`, setting `pending`,
  resetting `last_pipeline_run_at`. Per-picture MIME preflight (rows that can't embed → not
  enqueued, status `unsupported`).
- Enqueue reconcile jobs per §5; wake the pipeline once.
- Response: `{ updated, jobs: [job_id…], unsupported: [picture_id…] }`. Per-picture
  progress is then observable via `exif_sync_status`.

### 7.3 Field clear semantics

`set` writes only the named fields (others unchanged); `clear` lists fields to null out
(row column → NULL / JSONB key removed; file tag deleted on reconcile). Three states —
absent (unchanged), in `set`, in `clear` — avoid the `Option<Option<T>>` ambiguity. A
field present in both `set` and `clear` is a `400`.

**Editable field set** (the existing `ExifOverrides` fields): `captured_at`, `gps_lat`,
`gps_lng`, `gps_alt`, `orientation`, `camera_brand`, `camera_model`, `focal_length_mm`,
`f_number`, `iso_speed`, `exposure_time_num`, `exposure_time_den`. Validators: GPS bounds
(`lat ∈ [-90,90]`, `lng ∈ [-180,180]`); clearing GPS clears lat+lng+alt together;
orientation ∈ 1..=8.

### 7.4 `EditPictureConfig` evolution (`common/src/job.rs`)

Replace `exif_overrides: Option<ExifOverrides>` with an explicit edit delta + revert
baseline:

```rust
pub struct EditPictureConfig {
    pub picture_id: Uuid,
    pub exif: Option<ExifEdit>,         // None for a pure visual job
    pub visual: Option<VisualTransformations>,
}

pub struct ExifEdit {
    pub set: ExifOverrides,             // only Some fields are written
    pub clear: Vec<ExifField>,          // fields to delete
    pub previous: ExifSnapshot,         // prior value of every field in set ∪ clear,
    // distinguishing "had value V" from "was absent"
}
```

`ExifField` = an enum of the editable fields. `ExifSnapshot` stores, per touched field,
`Some(value)` or an explicit "was absent" — used only by the backend's revert (§4.3), not
the worker. Keep `ExifOverrides` for `set`. Update the round-trip tests in
`common/src/job.rs`.

Repository: keep `apply_exif_overrides` (forward apply of `set`/`clear`); add
`revert_exif(picture_id, previous, expected_new)` that restores `previous` **only if** the
row still equals `expected_new` (value-gated, §4.3).

### 7.5 Sync status & resync

- Picture details/list expose `exif_sync_status`.
- `POST /api/authenticated/pictures/{id}/exif/resync` — re-enqueue for a picture stuck in
  `pending` with no in-flight job (manual trigger for the rare crash-mid-completion case).

## 8. MIME preflight

At edit time (before committing Phase 1), screen the picture's `mime_type` with the
worker crate's `supports_exif()` / `MIME_TYPES_EXIF` logic (lift the predicate into
`common` or `domain` so the backend can call it). If unsupported: commit the DB edit, set
`exif_sync_status = unsupported`, enqueue no job. Never enqueue a reconcile that is doomed
to fail permanently.

## 9. Versioning policy

Snapshot the current `pictures`-bucket file into the `versions` bucket **before** the
reconcile overwrites it, per this predicate (evaluated at job claim, where versioning runs
today):

```
snapshot_version =
    match mode {
        None           => false,
        OriginalCopy   => !has_existing_version,          // keep only the original, once
        FullVersioning => !has_existing_version || is_visual_edit,
    }
```

- **None** → never. **OriginalCopy** → exactly one version (the pristine original, captured
  on the first edit of any kind). **FullVersioning** → original + one snapshot per visual
  edit; exif-only edits add none.
- `has_existing_version` = `PictureVersionRepository` has ≥1 row for the picture.
  `is_visual_edit` = `config.visual.is_some()`.
- The first-edit snapshot captures the still-pristine original (nothing has overwritten the
  bucket yet), so "keep the original" holds.
- **MVP note:** with no visual edits implemented, `FullVersioning` and `OriginalCopy`
  behave identically (both keep just the original); the `is_visual_edit` branch is wired
  for when visual edits land.

Fix the current claim-path logic (`api/worker/handlers.rs`), which snapshots on every edit
for both non-`None` modes and ignores visual-vs-exif. Add a
`PictureVersionRepository::has_versions(picture_id) -> bool` helper.

## 10. Federation

### 10.1 Extend the announced picture

`AnnouncedPicture` (`clients/federation/models.rs`) carries only
`filename, mime_type, file_size, width, height, captured_at`. Add `gps_lat, gps_lng,
gps_alt, orientation, exif_data`. Populate them in `AnnouncedPicture::from_picture`.

### 10.2 Extend the recipient write path

`ReceivedPictureInfo` (`services/shares/registration.rs`) and
`PictureRepository::create_received` accept and persist the new fields; the
`ON CONFLICT DO UPDATE` refreshes them so a re-announce updates the recipient row. This
also fixes recipient-side `gps_within_bbox` tagging on shared pictures (previously NULL).

### 10.3 Propagate metadata edits to recipients

The pipeline announcement step (`infra/pipeline/announcement.rs::reconcile_active_batch`)
today announces only *new* coverage and token moves. Extend it to **re-announce
already-tracked pictures whose metadata changed**, gated to avoid chatter:

- Gate on `pictures.updated_at > share_announcements.announced_updated_at`. EXIF edits bump
  `updated_at` (§4.1); pure tag changes do not touch the `pictures` row, so they don't
  trigger re-announce. (Verify no tag/pipeline write bumps `pictures.updated_at`.)
- On successful (re-)announce, set `announced_updated_at = pictures.updated_at`.
- `create_received`'s idempotent upsert (§10.2) makes re-delivery safe.
- Only the owner's edits propagate; relayers forward the owner's identity unchanged, and a
  relayer's row for an upstream picture is refreshed when the upstream re-announces — edits
  flow owner → … → leaf.

This reuses the existing deliver-then-record machinery; no new federation verb.

## 11. Edge cases

1. **Received pictures are read-only** — rejected in `enqueue_edit_for_user` today; the
   batch endpoint enforces it per-id; UI hides the action.
2. **Edit during initial extraction.** A picture whose initial `gen_thumbnail` (EXIF
   extraction) has not completed has no authoritative DB EXIF yet. Reject edits with `409`
   ("picture still processing") until `thumbnails_generated_at IS NOT NULL`, to avoid the
   extraction racing/overwriting the user's edit. **Exception:** the gate only applies to
   formats the worker extracts from (`supports_exif || supports_thumbnail`, unknown MIME
   included). A format the worker touches for neither never gets `thumbnails_generated_at`
   stamped, so gating on it would permanently block its (DB-only, `unsupported`) edit — those
   are allowed straight through.
3. **`captured_at` change** moves a picture between segments — handled by the §3.1 pipeline
   wake; overlap warnings still apply.
4. **Clearing a pipeline-relevant field** (e.g. GPS) drops dependent rule tags
   automatically — pipeline tags are live and re-derived once the picture is dirtied.
5. **Timezone.** `captured_at` is `NaiveDateTime`; edits use the same naive convention as
   extraction. `OffsetTimeOriginal` is not modelled (future field).
6. **Orientation** changes rendering without re-encoding pixels; thumbnails must respect it
   — treat an orientation change as needing thumbnail regen.
7. **Partial batch failure** — validate-all-then-apply the DB write atomically; individual
   reconcile jobs fail independently and surface via `exif_sync_status`; they never fail the
   committed DB edit.
8. **Idempotency / double-submit** — DB edits are last-writer-wins; the §5 in-flight rule
   and value-gated revert keep retries from multiplying rewrites or corrupting state.

## 12. Documentation updates

- `doc/01_GENERAL_SPECIFICATIONS.md` — the `metadata` trigger label now actually fires on
  EXIF edits (was a no-op).
- `doc/03_BACKEND_ARCHITECTURE.md` — new batch endpoint + `exif_sync_status`; announcement
  now carries gps/exif/orientation; `reconcile_active_batch` re-announces on metadata change.
- `doc/04_WORKER_ARCHITECTURE.md` — `edit_picture` reorder (upload original last),
  full-field EXIF write + clear, versioning predicate, MIME preflight moved server-side.
- **Consistency sweep:** confirm no doc claims the `pictures` bucket is *immutable after
  upload* — it is overwritten in place on edit, with the prior file copied to `versions`.
  `back/.env.example` is already corrected; check `02_INFRASTRUCTURE_DESIGN.md`,
  `03_BACKEND_ARCHITECTURE.md`, `04_WORKER_ARCHITECTURE.md`, and S3 module comments. Correct
  to: pictures = current/latest (mutable); versions = previous versions + preserved original.

## 13. Work breakdown

- [ ] **Fixes first:** pipeline wake + `last_pipeline_run_at` reset on edit; worker
  upload-original-last reorder; `write_exif_overrides` full-field coverage + clear;
  `OriginalCopy`/versioning predicate fix.
- [ ] Schema: `picture_exif_sync_status` + column; `uq_edit_picture_inflight`;
  `share_announcements.announced_updated_at`; `sqlx prepare`.
- [ ] `EditPictureConfig`/`ExifEdit`/`ExifSnapshot` (`common`); `apply_exif_overrides`
  delta apply + `revert_exif` (value-gated); round-trip tests.
- [ ] MIME preflight predicate shared with backend (§8).
- [ ] Single edit handler → write-through Phase 1; concurrency rule (§5) in the
  enqueue/complete paths; `exif_sync_status` in responses; `…/exif/resync`.
- [ ] Batch endpoint `PATCH /pictures/exif` (chunked, no cap, per-id validation + preflight).
- [ ] Federation: extend `AnnouncedPicture` + `create_received` + `ReceivedPictureInfo`;
  `reconcile_active_batch` metadata re-announce gated on `updated_at`.
- [ ] (Optional) stuck-`pending` recurring sweep via the framework in spec 03.
- [ ] Validators: GPS bounds, orientation range, owned-only, set/clear conflict, edit-during-
  processing 409.
- [ ] Tests: pipeline re-eval on date/GPS edit; revert on permanent failure + value-gating;
  chained-edit convergence across failures; recipient receives geo/exif; metadata edit
  re-announced (and *not* re-announced on tag-only change); versioning policy per mode;
  MIME-unsupported → `unsupported`; received-picture edit rejected.
- [ ] Docs (§12), including the immutable-bucket consistency sweep.
