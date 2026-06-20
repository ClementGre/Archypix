# Trash, Owner-Deletion Propagation & Recipient EXIF Overrides

## 1. Overview & goals

Roadmap item **"Trash & restore"** ([99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md)), expanded to cover the
way deletion and received-picture EXIF interact with sharing and transitive sharing:

1. **Trash & restore** — soft delete (`deleted_at`) for owned and received pictures, with
   user-configured retention driving physical **purge** of *owned* pictures.
2. **Owner-deletion propagation** — when an owner trashes a shared picture, recipients keep it during
   a grace window with a "will be deleted on *X*" warning instead of it vanishing; on purge it is
   unannounced.
3. **Recipient EXIF overrides** — a recipient may locally customise the EXIF of a received picture
   (add missing geo, fix `captured_at`) **without** mutating the owner's data, while still tracking
   the owner's authoritative value field-by-field.

Two adjacent features are specified separately but **share this feature's single schema change** (§4):

- **Recipient EXIF *editing* with owner propagation** (share-level permission, propose-to-owner
  endpoint, owner auto-apply + re-announce) → [10_recipient_exif_editing.md](10_recipient_exif_editing.md).
- **Physical copy ("rescue") + content dedup** (copy endpoint, `content_hash`, the dedup reconciler,
  boomerang handling) → [11_physical_copy_and_dedup.md](11_physical_copy_and_dedup.md).

This doc references those only where needed. The local-override path here is the **fallback** the
recipient uses when owner editing is not authorised (§6.2).

## 2. The invariant

Everything below follows from one rule:

> **A backend relays the *owner-authoritative* state of a picture — its deletion lifecycle and its
> EXIF. A recipient's local modifications (local trash, local EXIF overrides) are private to that
> recipient: they never propagate, in either direction, and never affect share coverage.**

Consequences that resolve the hard cases up front:

- **Which trash/EXIF state does a transitive share announce — the relayer's or the owner's?** Always
  the owner's. A relayer (B in A→B→C) forwards A's authoritative state; B's own local trash/overrides
  stay on B's backend.
- **Double receipt (A→C *and* A→B→C) can't contradict itself.** Both paths carry A's authoritative
  state, so they write the *same* values to C's single deduped row — idempotent, no back-and-forth.
- **The owner is the serialization point for edits.** Recipient edit-requests (spec 10) apply at the
  owner in arrival order; the owner's row is the single truth and re-announces — so last-write-wins is
  safe, no split brain.

## 3. Decisions (settled)

- **Owned vs received "trash" need no enum** — they are already **separate rows**
  (`pictures.local_user_id`; received rows link to the owner via `remote_picture_id`). The owner's
  `deleted_at` is on the owner row; the recipient's on the recipient row. We add a *propagated*
  owner-lifecycle field on received rows, distinct from the recipient's own `deleted_at`.
- **Owner-trash keeps the picture in share coverage** (re-announced with the lifecycle flag) until
  **physical purge**, the only moment it is unannounced. **Revoke remains the hard-remove**
  (immediate) — the escape hatch for "gone from recipients *now*".
- **Coverage is decoupled from local `deleted_at`.** A relayer's local trash of a received picture
  must not unannounce it downstream; coverage of relayed pictures is driven by tag membership.
- **Retention is finite.** Only `user_settings.trash_retention_days` (default 30). No infinite/archive
  option (that is a separate future concept).
- **Badge in normal view**, red, with the purge date/countdown. The recipient's *own* trash is the
  existing local trash view; the owner-deletion warning shows on the still-visible picture.
- **A deletion carries a reason** (`deleted_reason`). For now only `'manual'` is produced; the other
  values exist for spec 11 (`content_dedupe`, `boomerang`), so the deferred work needs no migration.
- **`local_exif_overrides` is a sparse per-field key set.** An owner update to a field the recipient
  did **not** override still flows through; an overridden field stays sticky. Effective EXIF =
  `merge(remote_exif_data, local_exif_overrides)` (override wins per field).
- **Recipient overrides are DB-only.** No `edit_picture` job, no file reconcile (the recipient does
  not own the file); `exif_sync_status` is meaningless for received rows.
- **No new federation verb in this doc.** Lifecycle + owner EXIF snapshot ride the existing
  announce / `picture_update` payloads (extends [04 §10](04_better_exif_support.md)).

## 4. Schema changes (single consolidated migration — covers 09, 10 & 11)

> This section is the **only** edit to `back/migrations/001_initial_schema.up.sql` for features 09,
> 10 and 11. Every column is added **now** so the migration is touched once; the [10]/[11] columns sit
> unused until those features ship. Apply inline in the table definitions (not trailing `ALTER`s) to
> match the file's style, then `cd back && cargo sqlx migrate revert && cargo sqlx migrate run &&
> cargo sqlx prepare -- --tests` and migrate the seeded test DBs per
> [00_CODING_GUIDELINES.md](../00_CODING_GUIDELINES.md). Tags: **[09]** trash + overrides,
> **[10]** recipient EXIF editing, **[11]** physical copy + dedup.

**New enum:**

```sql
-- [09] 'manual'; [11] 'boomerang', 'content_dedupe'
CREATE TYPE picture_deleted_reason AS ENUM ('manual', 'boomerang', 'content_dedupe');
```

**`pictures`** (received-only columns are NULL for owned rows):

```sql
owner_deleted_at           TIMESTAMP,                 -- [09] received: owner's soft-delete (announced)
owner_purge_at             TIMESTAMP,                 -- [09] received: announced purge deadline
remote_exif_data           JSONB,                     -- [09] received: owner's authoritative EXIF snapshot
local_exif_overrides       JSONB,                     -- [09] received: recipient sticky per-field overrides
deleted_reason             picture_deleted_reason,    -- [09/11] set with deleted_at; NULL when not deleted
content_hash               TEXT,                      -- [11] metadata-stripped content hash (dedup grouping)
copy_source_owner_username VARCHAR(255),              -- [11] physical copy provenance: original owner …
copy_source_owner_instance VARCHAR(255),              -- [11] … instance …
copy_source_picture_id     VARCHAR(255),              -- [11] … and original picture id (NULL unless a copy)
```

**Typed EXIF model.** Two structs in `archypix-common` (`domain::job` re-exports them): `CameraExif`
— the seven camera/lens fields — and `FullExif` (the five promoted fields **+** a flattened
`CameraExif`). `pictures.exif_data` is `Json<CameraExif>` — camera/lens **only**, for owned *and*
received rows; the five promoted fields always live in their own columns (`captured_at`,
`gps_lat/lng/alt`, `orientation`). `remote_exif_data` and `local_exif_overrides` are `Json<FullExif>`.
For a received row the effective EXIF is `remote_exif_data.merged_with(local_exif_overrides)`
(`FullExif::merged_with`, override key wins); its `camera` part is written to `exif_data` and its
promoted fields to the promoted columns, recomputed whenever `remote_exif_data` or
`local_exif_overrides` changes. So the pipeline and rule predicates keep reading `exif_data` + the
promoted columns unchanged. `Picture::full_exif()` reassembles a row's columns + `exif_data` back into
a `FullExif`.

**`user_settings`:**

```sql
trash_retention_days INT NOT NULL DEFAULT 30,         -- [09]
```

**`outgoing_shares`** and **`incoming_shares`:**

```sql
allow_exif_edit BOOLEAN NOT NULL DEFAULT FALSE,       -- [10] owner grants recipient EXIF editing;
                                                      --      propagated to the incoming_shares copy
```

**Indexes:**

```sql
CREATE INDEX idx_pictures_owned_trashed ON pictures (deleted_at)                 -- [09] purge sweep
    WHERE deleted_at IS NOT NULL AND remote_picture_id IS NULL;
CREATE INDEX idx_pictures_content_hash ON pictures (local_user_id, content_hash) -- [11] dedup grouping
    WHERE content_hash IS NOT NULL;
```

## 5. Trash & restore

### 5.1 Owned pictures

- **Delete** → set `deleted_at = now()`, `deleted_reason = 'manual'` on the owner row. Excluded from
  the owner's own views/WebDAV (existing behaviour) **but kept in share coverage** (§7). A recurring
  **purge sweep** ([03_recurring_tasks_framework.md](03_recurring_tasks_framework.md)) physically
  deletes the S3 objects + row once `deleted_at + trash_retention_days < now()`. `owner_purge_at` is
  **not stored** on the owner row — it is *derived* from `deleted_at + trash_retention_days` at
  announce/sweep time, so changing retention takes effect with no backfill.
- **Restore** → clear `deleted_at`/`deleted_reason`; re-dirty + wake the pipeline so the next
  reconcile re-announces with the flag cleared.
- **Physical purge** removes the picture row → `share_announcements` rows cascade-delete (tokens
  invalidated) → the announcement diff unannounces it from recipients.

### 5.2 Received pictures

- **Delete** → set `deleted_at`, `deleted_reason = 'manual'` on the recipient row. **Local only**:
  hides from the recipient's views, never physically deletes anything, never announced, **never
  affects downstream relay** (§7). Tag records are retained
  ([01_GENERAL_SPECIFICATIONS.md §2](../01_GENERAL_SPECIFICATIONS.md)).
- **Restore** → clear `deleted_at`/`deleted_reason`.
- **No auto-purge.** A received row is pure metadata; it lingers (hidden) until restore, or until the
  share is revoked / the owner purges (which removes the row via unannounce).

### 5.3 The three states a recipient sees

| recipient `deleted_at` | `owner_purge_at` | UI                                                                      |
|------------------------|------------------|-------------------------------------------------------------------------|
| NULL                   | set              | normal view **+ red badge** "owner will delete this on *X*" (countdown) |
| set                    | NULL             | in my trash; stays as long as the share lives                           |
| set                    | set              | in my trash; **+ "permanently gone on *X*"**                            |

## 6. EXIF: owner-authoritative snapshot + sticky local overrides

### 6.1 Model

For a received row:

- `remote_exif_data` — the owner's authoritative snapshot, refreshed on every announcement.
- `local_exif_overrides` — sparse; the explicit **set of fields** the recipient claimed, with their
  values. Store the explicit key set (not a diff of effective-vs-remote) so that an owner later
  setting a field to the recipient's value does not silently transfer ownership of that field.
- `exif_data` (+ promoted columns) = `merge(remote_exif_data, local_exif_overrides)`, recomputed on
  every announcement and on every override change.

A local override is a `metadata`-label event **locally** (re-runs rule/segment tagging on the merged
EXIF). It is DB-only — no file reconcile.

### 6.2 The two edit paths

- **Override locally** (this doc) → write the field into `local_exif_overrides`, recompute the merge,
  fire the local `metadata` event. Instant; diverges from the owner for that field. Always available.
- **Request owner edit** (only if the share authorises it — spec 10) → the owner applies the edit and
  re-announces; the value arrives in `remote_exif_data` and flows through (no override written). When
  escalating a field this way, **clear any existing override on that field first** so the owner's
  value is no longer shadowed. Higher latency (round-trip + re-announce), but stays in sync. Full
  mechanism, permissions and endpoints: [10_recipient_exif_editing.md](10_recipient_exif_editing.md).

### 6.3 Limitation

A recipient's override lives only in the DB; the owner's *file* is unchanged. Downloading the
**original** yields the owner's embedded EXIF, not the override. Injecting overrides into a downloaded
file is out of scope.

## 7. Coverage & announcement

The pipeline announcement step (`infra/pipeline/announcement.rs`) stays the single
picture-announcement path (deliver-then-record). Two changes:

1. **Coverage includes owner-trashed-pending pictures and ignores local trash.** The share-coverage
   set must **include** owned pictures with `deleted_at` set but not yet purged (carry the lifecycle
   flag), and **not exclude** relayed (received) pictures on the relayer's local `deleted_at`
   (membership is by tag). This replaces the blanket "exclude `deleted_at IS NOT NULL`" of
   `get_pictures_under_tag` for the *share-coverage* use; that helper stays as-is for plain
   library/WebDAV listing.

2. **The announced picture carries owner-authoritative lifecycle + EXIF**, forwarded verbatim by
   relayers:

   | Announced field | Owned row source | Received row source (relay) |
      |---|---|---|
   | `owner_deleted_at` | `deleted_at` | stored `owner_deleted_at` |
   | `owner_purge_at` | `deleted_at + trash_retention_days` (derived) | stored `owner_purge_at` |
   | `exif` (one typed `FullExif`) | `full_exif()` (columns + `exif_data`) | stored `remote_exif_data` |

   A relayer never announces its merged effective EXIF or its local `deleted_at` — only the owner
   snapshot it holds. This is the §2 invariant in code.

Re-announce gating reuses the existing
`share_announcements.announced_updated_at > pictures.updated_at` mechanism from
[04 §10.3](04_better_exif_support.md): trashing, restoring, and overriding all bump the relevant row's
`updated_at`, so they re-deliver; nothing new on the wire.

## 8. Federation

- **`AnnouncedPicture`** (`clients/federation/models.rs`) gains `owner_deleted_at`, `owner_purge_at`,
  and carries the owner EXIF as one flattened, typed `exif: FullExif` — `from_picture` uses
  `picture.full_exif()` for owned rows and the stored `remote_exif_data` for received rows.
- **Recipient write path** (`services/shares/registration.rs::ReceivedPictureInfo` +
  `PictureRepository::create_received`): persist `owner_deleted_at`/`owner_purge_at`, write the
  announced EXIF into **`remote_exif_data`** (not directly into `exif_data`), then recompute
  `exif_data = merge(remote_exif_data, local_exif_overrides)` and the promoted columns. The
  `ON CONFLICT DO UPDATE` refreshes `remote_exif_data` + lifecycle on every re-announce while
  **preserving `local_exif_overrides`**.
- **Revocation** is unchanged (local-first; removes `/SharedToMe/…` tags and unreachable rows) and is
  still the hard-remove distinct from owner-trash.

## 9. Edge cases

1. **Owner trashes a shared picture** → recipients keep it with the red badge during grace; on purge
   it is unannounced (vanishes). To remove it from recipients immediately, the owner **revokes**.
2. **Owner restores before purge** → re-announce clears `owner_deleted_at`/`owner_purge_at` and the
   badge.
3. **Recipient trashes a relayed picture (B trashes A's, B→C)** → local only; still relayed to C
   (coverage by tag, §7); C is unaffected.
4. **Owner edits a field the recipient overrode** → override wins (sticky). **Owner edits a
   non-overridden field** → flows through the merge.
5. **Retention setting changed** → purge timing is derived at sweep/announce time; pictures already in
   trash respect the new value (no backfill).
6. **Recipient trashed a received picture, then the owner purges it** → unannounce removes the row; it
   disappears from the recipient's trash too (the file is gone — "rescue" is spec 11).
7. **Owned picture in trash** still consumes the owner's storage until purge (relevant once quotas
   land).
8. **Non-thumbnailable / extraction-incomplete pictures** behave as today; overrides still merge over
   whatever `remote_exif_data` is present.

## 10. Future: physical copy of a picture

A later feature lets a recipient keep a picture the owner is about to purge by making a **physical
copy** into their own library (the file lives on the owner's storage, so "keeping" means copying the
bytes). A copy is a **new, independent owned picture** — distinct identity, never a reuse of the
original `picture_id`. Identical copies are de-duplicated for display via the `content_hash` +
`deleted_reason` machinery whose columns are already in §4. Full design, including the dedup
reconciler and the deleted-content "boomerang" guard:
[11_physical_copy_and_dedup.md](11_physical_copy_and_dedup.md).

## 11. Documentation updates

- [01_GENERAL_SPECIFICATIONS.md §2](../01_GENERAL_SPECIFICATIONS.md) — expand Deletion & Trash: owner
  vs recipient rows, owner-deletion propagation + grace window, revoke-as-hard-remove, retention.
- [03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md) — announcement carries
  `owner_deleted_at`/`owner_purge_at` + owner EXIF snapshot; coverage decoupled from local trash;
  received-row `exif_data` is a materialised merge; purge sweep added to the scheduler.
- `user_settings` docs — `trash_retention_days`.

## 12. Work breakdown (this feature)

- [x] Schema: the full consolidated §4 (so 10/11 need no further migration); seeded-DB migration +
  checksum fix; `sqlx prepare`.
- [x] Domain: EXIF merge helper (`domain/received_exif.rs`: `materialize(remote, overrides)` +
  promoted-column recompute, `build_owner_exif`/`decompose`); override set/clear with per-field key
  tracking.
- [x] Trash API: delete/restore for owned & received (received delete is local-only; owned delete
  keeps share coverage; both set `deleted_reason = 'manual'`).
- [x] Purge sweep recurring task (owned, `deleted_at + retention < now`, retention derived per-owner)
  → unannounce + tracking delete, S3 + row delete.
- [x] Coverage change: the share-coverage query already includes owner-trashed-pending owned pictures
  and ignores relayer local `deleted_at` (it never filtered `deleted_at`); `get_pictures_under_tag`
  left as-is for library/WebDAV listing.
- [x] Announcement: `AnnouncedPicture` lifecycle fields + owner-snapshot EXIF selection (`from_picture`
  decomposes `remote_exif_data` for relays); owner_purge_at derived in the pipeline step.
- [x] Recipient write path: `create_received` writes `remote_exif_data` + lifecycle and preserves
  `local_exif_overrides`; `apply_received_materialization` recomputes the merge on upsert.
- [x] Local override endpoint (DB-only `metadata` event; per-field set/clear). The "propose to owner"
  escalation lives in spec 10.
- [x] Settings: `trash_retention_days` read/write + validation (1–3650).
- [x] Tests (`tests/trash_and_overrides.rs` + `domain/received_exif.rs` unit tests): owner-trash grace
    + badge fields announced; purge removes the owned row + tracking; restore clears; recipient local
      trash does not drop coverage; override sticky per-field; owner edit flows through a non-overridden
      field; retention change affects derived purge; received override never enqueues `edit_picture`.
- [x] Docs (§11).
