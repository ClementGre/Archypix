# Feature 22 — Storage Quotas

## 1. Overview & goals

Per-user storage quotas: cap how many bytes a user's **owned** content occupies, enforce that cap on
every byte-adding action (upload, WebDAV `PUT`, physical copy), and surface usage to the user, to the
admin, and to WebDAV clients (RFC 4331 capacity bar).

Two things the system must do well:

- **Enforce cheaply.** A quota check must be O(1), never a `SUM(file_size)` scan on the hot path.
- **Account honestly.** "Used" must reflect the bytes actually stored on the user's own S3 keys —
  originals + versions, live **and** trashed (trash still occupies S3 until the purge sweep) — and
  must **exclude** received/shared pictures (their bytes live on the owner's backend; the owner pays).

Thumbnails are **not billed** (free platform overhead), but their real byte size can still be
reported to admins on demand by measuring the S3 prefix (§8.3) — they are not tracked in the DB.

Non-goals: quota on job/queue usage, bandwidth quotas, resolver-authoritative quota (the resolver
only *seeds* a default — §9).

---

## 2. Decisions

1. **Billed = owned originals + owned versions, each split live / trashed** — four categories. A
   received picture (`pictures.remote_picture_id IS NOT NULL`) is never billed. Trashed content stays
   billed until the purge sweep frees its S3 objects (09 §5.1) — this is what powers "empty trash to
   reclaim X".
2. **Thumbnails are free**, and untracked in the DB. Admin can measure them via an S3 prefix walk
   (§8.3); never enforced.
3. **Authoritative usage lives in Postgres** (`user_storage`, §4.2), maintained by **triggers** so it
   stays correct across *every* code path that touches `file_size`, `deleted_at`, or
   `remote_picture_id` — no per-call-site bookkeeping to forget. Respects the "Postgres is the source
   of truth" invariant (02).
4. **Redis holds the fast-path values**: a cached mirror of the committed billed total plus the
   in-flight **reservation** counter (§5). Enforcement math = `committed + reserved + incoming`.
5. **A reconcile routine** recomputes the four counters from scratch on an interval — the drift safety
   net that lets the fast counter be trusted, and refreshes the Redis mirror.
6. **Quota authority is the backend** (`users.storage_quota_bytes`), admin-updatable. The resolver may
   seed the initial value at provisioning; it is not authoritative and does not override the admin.
7. **Over-quota policy:** block byte-adding writes at 100 % (`507`/`413`); reads, deletes, trash and
   restore always work. Soft warnings at 80 % / 90 % surfaced in `GET /me/storage`.
8. `NULL` quota = **unlimited**. New users default to `config.default_storage_quota_bytes`
   (`NULL`/`0` ⇒ unlimited).

---

## 3. What counts (accounting model)

| Category          | Source rows                                                          | Billed |
|-------------------|----------------------------------------------------------------------|:------:|
| Live originals    | `pictures` — `remote_picture_id IS NULL`, `deleted_at IS NULL`       |   ✅    |
| Trashed originals | `pictures` — `remote_picture_id IS NULL`, `deleted_at IS NOT NULL`   |   ✅    |
| Live versions     | `picture_versions` of an owned picture with `deleted_at IS NULL`     |   ✅    |
| Trashed versions  | `picture_versions` of an owned picture with `deleted_at IS NOT NULL` |   ✅    |
| Received pictures | `pictures` — `remote_picture_id IS NOT NULL`                         |   ❌    |
| Thumbnails        | S3 thumbnail bucket, key prefix `{user_id}/` — untracked in DB       |   ❌    |

`file_size` is `NULL`/provisional between row creation and worker completion; the trigger treats
`NULL` as `0`, so in-flight pictures under-count briefly and the delta lands when the worker writes the
authoritative size. For direct uploads the size is already authoritative at row creation
([`complete_upload`](../../back/src/services/pictures.rs) reads it back from an S3 HEAD), so there is no
window there.

**Headline numbers for UI:** *Used* = all four cells; *Reclaimable (trash)* = trashed originals +
trashed versions.

---

## 4. Schema changes (`0007_storage_quotas`)

### 4.1 Quota column

```sql
ALTER TABLE users
    ADD COLUMN storage_quota_bytes BIGINT; -- NULL = unlimited
```

### 4.2 Usage breakdown table

```sql
CREATE TABLE user_storage
(
    user_id                 UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    originals_bytes         BIGINT    NOT NULL DEFAULT 0, -- live owned originals
    originals_trashed_bytes BIGINT    NOT NULL DEFAULT 0, -- soft-deleted owned originals
    versions_bytes          BIGINT    NOT NULL DEFAULT 0, -- versions of a live owned picture
    versions_trashed_bytes  BIGINT    NOT NULL DEFAULT 0, -- versions of a trashed owned picture
    updated_at              TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'utc')
);
```

`billed_total = originals_bytes + originals_trashed_bytes + versions_bytes + versions_trashed_bytes`
(a generated helper or computed in the repo). One row per user, upserted lazily by the trigger; the
migration backfills every existing user (§4.4).

### 4.3 Triggers (§5 details the deltas)

- `AFTER INSERT OR DELETE OR UPDATE OF file_size, deleted_at, remote_picture_id ON pictures`
- `AFTER INSERT OR DELETE OR UPDATE OF file_size ON picture_versions`
- `AFTER UPDATE OF deleted_at ON pictures` also moves that picture's **version** bytes between the
  live/trashed version buckets (versions inherit their parent's trash state).

All row-level; a `PLpgSQL` function per table applying signed deltas to the owner's `user_storage`
row (`INSERT … ON CONFLICT DO UPDATE`).

### 4.4 Backfill

```sql
INSERT INTO user_storage (user_id, originals_bytes, originals_trashed_bytes,
                          versions_bytes, versions_trashed_bytes)
SELECT u.id, ... -- SUM(file_size) FILTER (...) over owned pictures + their versions, split by deleted_at
    FROM users u ...
    ON CONFLICT (user_id)
DO UPDATE SET ...;
```

---

## 5. Delta accounting & the upload race

### 5.1 Trigger deltas

Each trigger computes the **old contribution** and **new contribution** of the changed row and applies
`new − old` to the correct bucket(s). This one rule covers every case uniformly:

- **Upload / copy insert** → `+file_size` to `originals_bytes`.
- **Worker writes real `file_size`** (`UPDATE OF file_size`) → `+(new − old)`.
- **Trash** (`deleted_at` set) → move `file_size` from `originals_bytes` → `originals_trashed_bytes`,
  and the picture's version bytes from `versions_bytes` → `versions_trashed_bytes`.
- **Restore** → the reverse.
- **Purge** (`hard_delete`, `DELETE`) → subtract from the trashed buckets.
- Rows with `remote_picture_id IS NOT NULL` contribute `0` (received — never billed).

`picture_versions` rows pick their bucket from a lookup of the parent's `deleted_at`
(`SELECT deleted_at FROM pictures WHERE id = NEW.picture_id`); since most pictures have zero versions
this subquery is cheap even under the batch set-based UPDATEs used by trash/restore.

### 5.2 Redis fast path

- `storage:committed:{user_id}` — cached mirror of `user_storage.billed_total`. Read on the hot path;
  on miss, recompute from Postgres and repopulate. Refreshed at known write points and by the
  reconcile routine (§7).
- `storage:reserved:{user_id}` — sum of declared sizes for **in-flight presigned uploads** whose bytes
  are not yet in Postgres. Incremented at presign, decremented at `complete_upload` (or on session
  expiry). One sub-key per upload session (mirrors `UploadSession`'s TTL) so an abandoned upload
  auto-releases; the reserved value is the sum of live sub-keys.

**Effective usage** for any check = `committed + reserved`.

### 5.3 The race

Presigned uploads PUT to S3 staging *before* `complete_upload`, so N parallel presigns could each pass
an isolated check yet collectively exceed the quota. Reservations close the gap:

1. **Presign** (`begin_upload` / `begin_upload_batch`): reject if
   `committed + reserved + Σ declared_sizes > quota`; otherwise add a reservation per slot. The batch
   presign slot gains an **optional `size`** field to enable this; when absent, only the coarse
   `committed ≥ quota` gate applies and the hard check (below) is the backstop.
2. **`complete_upload`**: the authoritative S3 size is known → hard check
   `committed + size > quota`; if over, delete the promoted object, abort the row, release the
   reservation, return `413`. On success the trigger commits the size (via the INSERT) and the
   reservation is released.

---

## 6. Enforcement points

| Path                           | When size is known                | Action on over-quota                      |
|--------------------------------|-----------------------------------|-------------------------------------------|
| `begin_upload` / batch presign | client-declared (optional)        | reject presign `413`; reserve on success  |
| `complete_upload`              | authoritative S3 HEAD             | delete object, abort, release, `413`      |
| WebDAV `PUT`                   | `Content-Length` / streamed bytes | `507 Insufficient Storage` before promote |
| `copy_picture`                 | `source.file_size` (upfront)      | `507` before the S3 copy                  |

WebDAV overwrite (which may snapshot a version, 06 §7.3) checks the **projected net delta**
`new_size − old_size + snapshot_bytes`. Trash, restore, and delete are **byte-neutral or freeing** and
are never blocked. Reads are never blocked.

A shared helper `services::storage::check_and_reserve(user, incoming) -> Result<Reservation, AppError>`
centralises the effective-usage math so all four call sites behave identically.

---

## 7. Reconcile routine

A sweep-only [`Routine`](../../back/src/infra/routine.rs) (same shape as `purge_sweep`) that, per user
(batched), recomputes the four counters with a single grouped query over `pictures` + `picture_versions`,
writes `user_storage`, and refreshes `storage:committed:*`. Runs on an interval
(`storage_reconcile_interval_secs`, default daily) and corrects any trigger drift. Cheap: it is a set
of `SUM … GROUP BY user_id` scans, not per-object work.

---

## 8. API

### 8.1 User — `GET /api/me/storage`

```ts
{
    quota_bytes: number | null;          // null = unlimited
    used_bytes: number;                  // billed total
    available_bytes: number | null;      // null when unlimited
    breakdown: {
        originals_bytes: number;
        originals_trashed_bytes: number;
        versions_bytes: number;
        versions_trashed_bytes: number;
    }
    ;
    reclaimable_trash_bytes: number;     // originals_trashed + versions_trashed
    usage_ratio: number | null;          // used / quota, null when unlimited
    warn_level: "ok" | "warn" | "critical" | "full";  // 80% / 90% / 100%
}
```

Drives the account-page storage bar, the "empty trash to reclaim X" prompt, and the upload preflight.

### 8.2 WebDAV PROPFIND — RFC 4331 (06 §6)

On the collection PROPFIND, add live-properties:

- `{DAV:}quota-used-bytes` = billed total.
- `{DAV:}quota-available-bytes` = `max(0, quota − used)`; when unlimited, omit or return a large
  sentinel per RFC 4331.

Finder / Windows Explorer then render a native capacity bar for the mounted drive.

### 8.3 Admin

- **`PATCH /api/admin/users/{id}`** — extend with `storage_quota_bytes?: number | null` (set/clear the
  cap; `null` = unlimited). Lowering below current usage leaves stored bytes intact and blocks new
  writes until the user frees space.
- **`GET /api/admin/users` / `GET /api/admin/users/{id}/stats` / `GET /api/admin/stats`** — replace the
  live `SUM(file_size)` ([admin.rs](../../back/src/repository/admin.rs)) with the maintained counter,
  and add `quota_bytes`, the four-cell breakdown, and `usage_ratio` to the per-user payloads;
  `total_storage_bytes` becomes the sum of billed counters.
- **`GET /api/admin/users/{id}/storage-audit`** — the S3 truth check. Walks the `{user_id}/` prefix in
  each bucket (pictures, versions, thumbnails, staging) and returns measured `(object_count, bytes)`
  per bucket, alongside the DB breakdown and a **drift** figure (DB billed − S3 measured for
  originals+versions). This is the **only** way to see thumbnail bytes (untracked) and doubles as a
  per-user reconcile cross-check. On-demand and **Redis-cached** (listing cost scales with object
  count); it cannot split live vs trash (both share the prefix) — the DB counter covers that.

New `Storage` trait method:

```rust
async fn prefix_usage(&self, bucket: &str, prefix: &str) -> Result<PrefixUsage, AppError>;
// ListObjectsV2 paginated (1000/page), summing `Size`; returns { object_count, total_bytes }.
```

---

## 9. Resolver (optional seed)

The resolver may carry a per-user default plan so signups start on the right tier:

- Add nullable `quota_bytes` to `user_mappings` (resolver DB) and to the provisioning/update payload.
- On user creation the backend adopts the resolver-supplied value (if any) as the initial
  `users.storage_quota_bytes`, else `config.default_storage_quota_bytes`.

The backend remains authoritative; an admin change never round-trips to the resolver. This section is
independent of the core feature and can ship in a later phase.

---

## 10. Config (`infra/config.rs` + `.env.example`)

| Field                              | Env var                            | Default              | Meaning                                             |
|------------------------------------|------------------------------------|----------------------|-----------------------------------------------------|
| `default_storage_quota_bytes`      | `DEFAULT_STORAGE_QUOTA_BYTES`      | `0` (unlimited)      | Initial quota for new users when unset by resolver. |
| `storage_reconcile_interval_secs`  | `STORAGE_RECONCILE_INTERVAL_SECS`  | `86400`              | Reconcile-routine period.                           |
| `storage_reservation_ttl_secs`     | `STORAGE_RESERVATION_TTL_SECS`     | = upload-session TTL | Reservation sub-key TTL (auto-release).             |
| `storage_warn_ratio` / `_critical` | `STORAGE_WARN_RATIO` / `_CRITICAL` | `0.8` / `0.9`        | Warning thresholds in `GET /me/storage`.            |

Mirror the new vars into `Config::test_default()`.

---

## 11. Frontend

- **Account / settings page**: storage bar (used / quota), four-cell breakdown, "empty trash to
  reclaim X GB" action (links to trash purge).
- **Upload preflight**: read `GET /me/storage`; warn at `warn`/`critical`, block the picker at `full`
  with a clear message; surface `507`/`413` from `PUT`/complete as a friendly over-quota error.
- **Admin panel**: per-user quota editor (with unlimited toggle), usage bar + breakdown, and a
  "Storage audit" button that calls the S3-audit endpoint (shows thumbnails + drift).
- **WebDAV**: capacity bar comes for free from §8.2.

---

## 12. Edge cases

- **Received / shared pictures** never counted (owner pays) — advertised as a federation benefit.
- **Provisional `NULL` `file_size`** counted as `0`; corrected when the worker writes the real size
  (delta) and by reconcile.
- **Admin lowers quota below usage** → user is over; new writes blocked, nothing deleted, reads/deletes
  fine. `available_bytes` clamps at `0`.
- **Restore** is byte-neutral (trashed bytes were already billed) → never quota-gated.
- **Dedup / boomerang siblings** (feature 11) are soft-deleted but retain S3 objects until purge →
  billed as trashed, consistent with the "trash still costs" rule.
- **Purge sweep** frees bytes via `hard_delete` → the `DELETE` trigger decrements automatically; no
  extra bookkeeping in `purge_sweep`.
- **WebDAV overwrite** larger than the old file → net-delta check; a version snapshot adds its bytes.
- **Redis cold / evicted** → `committed` miss recomputes from Postgres; reservations lost on flush are
  bounded by TTL and reconverged by the hard check at `complete_upload`.
- **Trigger vs batch ops**: trash/restore/edit use set-based UPDATEs → row triggers fire per row; keep
  trigger bodies minimal (the version subquery is the only join, cheap for the common zero-version
  case).

---

## 13. Testing

- Trigger correctness: insert/update-size/trash/restore/purge/copy each move the right cell; received
  rows contribute 0; a version follows its parent's trash state.
- Reconcile equals trigger counters on a seeded dataset (drift = 0).
- Reservation race: concurrent presigns respect `committed + reserved`; `complete_upload` hard-check
  rejects and cleans up the S3 object; abandoned session releases on TTL.
- Enforcement at all four call sites returns the right status and leaves no orphan bytes.
- PROPFIND emits valid RFC 4331 properties (used/available); unlimited case.
- Admin: quota PATCH, breakdown in list/stats, S3 audit returns per-bucket bytes and a plausible drift.

---

## 14. Doc updates

- **06_API_REFERENCE.md** — `GET /me/storage`; `storage_quota_bytes` on `PATCH /admin/users/{id}`;
  `quota_bytes`/breakdown on admin user/stats payloads; `GET /admin/users/{id}/storage-audit`; RFC 4331
  props in the WebDAV section.
- **06_webdav.md §6** — quota properties in PROPFIND.
- **09_trash_and_exif_overrides.md** — note trash counts against quota until purge.
- **11_physical_copy_and_dedup.md** — copies are billed; dedup siblings billed as trashed until purge.
- **03_BACKEND_ARCHITECTURE.md** — the `user_storage` triggers + reconcile routine; `Storage::prefix_usage`.
- **99_ROADMAP_MVP.md** — mark **Storage quotas** in progress/done.

---

## 15. Work breakdown

1. [x] Migration `0007_storage_quotas`: `users.storage_quota_bytes`, `user_storage`, triggers, backfill;
   `cargo sqlx prepare`. (Delete accounting uses a `BEFORE DELETE` trigger on `pictures` so a
   picture's version bytes are read before the FK cascade removes them.)
2. [x] `UserStorageRepository` (read breakdown, reconcile query) + `services::storage`
   (`fits`/`at_or_over_quota`/`reserve`/`release`, effective-usage math, Redis keys). The effective
   math is centralised; the four call sites choose `413` (uploads) vs `507` (WebDAV/copy) per §6.
3. [x] Enforcement at `begin_upload`/`begin_upload_batch` (presign reservation), `complete_upload`
   (authoritative hard check), WebDAV `PUT` (net-delta), `copy_picture` (upfront).
4. [x] Reconcile `Routine` (`storage_reconcile`) + config + `main.rs` wiring; default-quota seed on
   user creation (`create_user`).
5. [x] `Storage::prefix_usage` (ListObjectsV2) + admin storage-audit endpoint (cached).
6. [x] `GET /me/storage`; admin quota PATCH; swap admin SUMs for the counter; add breakdown/quota to
   payloads.
7. [x] WebDAV RFC 4331 PROPFIND properties (collection `quota-used-bytes`/`quota-available-bytes`).
8. [x] Frontend: footer storage bar (colour by warn level), settings breakdown + reclaim-trash
   prompt, upload preflight (declared size, warn/block, friendly `413`/`507`). (Admin quota
   editor/audit UI intentionally deferred.)
9. [ ] (Optional, later) resolver `quota_bytes` seed.
10. [x] Tests (`tests/storage_quotas.rs`: triggers, reconcile, effective math, complete hard check);
    doc updates (§14).
