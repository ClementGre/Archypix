# Feature 36: GPS accuracy

## 1. Overview & goals

A location set by the fix tools (feature 30) is often a guess: an interpolation between two photos
hours apart, or a copy from a friend's picture. Once written it was indistinguishable from a
measured fix. Goal: record **how far off a location may be**, carry it through every EXIF path, and
let the fix tools set it deliberately.

Out of scope: a rule/predicate field on the accuracy (a natural "approximate location" rule, not
built), and showing the radius on the gallery map.

## 2. Decisions (settled)

- **A radius in metres, not a boolean.** `pictures.gps_accuracy_m double precision`, `NULL` =
  unstated, `0` = exact. "Approximate" is a threshold the UI can apply; a boolean cannot become a
  radius, and the fix tools already compute the number that sets it (§4).
- **The standard tag, write-through.** EXIF 2.31 `GPSHPositioningError` (GPS IFD 0x001F, a rational
  in metres) — portable, read by other tools, survives export. It joins `FullExif` like every other
  editable field, so the `file_exif` snapshot, diff badges, revert and federation need no special
  case. Rejected: a DB-only flag (breaks the one-model-for-every-field invariant of 31 §3) and
  truncating decimals (lossy, and indistinguishable from a genuinely coarse fix).
- **Centimetre precision.** Read paths round to 2 decimals and the writer stores `round(m·100)/100`,
  so a value set through the UI reads back identically (no permanent diff badge).
- **One-way coupling.** Clearing a coordinate clears the accuracy (a radius around nothing is
  meaningless); clearing the accuracy alone keeps the location. `couple_gps` pulls the accuracy into
  a coordinate clear, never the reverse.
- **Range** `[0, 20 000 km]` — half the Earth's circumference, and what the centimetre rational fits
  in an `i32`.

## 3. Read & write paths (worker)

- **Read.** rexiv2 `Exif.GPSInfo.GPSHPositioningError`, ExifTool `-GPSHPositioningError#`; not gated
  on the coordinates, so an accuracy-only file round-trips exactly. `drop_null_island` drops it with
  the rest of the group (30 §12.11).
- **Write.** Its own tag: set when `Some`, deleted otherwise (`target_clear_fields` lists it
  independently of the coordinate group). The rexiv2 writer now **clears before it sets**:
  `delete_gps_info` wipes the whole GPS IFD, which would otherwise erase an accuracy written in the
  same pass. Set and clear lists are disjoint, so the reorder changes nothing for other fields.

## 4. Fix tools

`deriveGps` returns a **suggested** radius alongside the point: the worst case, given the true
location lies among the sources — the distance to the farthest source plus that source's own
accuracy, in whole metres. A copy from one photo inherits that photo's accuracy.

`GpsAccuracyControl` (single-target `GpsFixPanel` and batch `BatchReferencePanel`) holds an
`AccuracyChoice` — **Suggested**, **Exact** (0), **Same as photo(s)** (the largest accuracy the
sources state), or a custom value. A mode rather than a value, so "Suggested" follows the derivation
as its anchors load. The radius is drawn around the pin (`MapView.pointRadiusM`).

A fix that sets a location without an accuracy **removes** the target's stale one (owned `clear`,
received `empty`): it described the old location.

The general editors expose it too: an accuracy input in `GpsPickerPopover` (inline editor, bulk
preview rows), and a separate batch row so a selection can be marked approximate without moving it.

## 5. Populating existing pictures: the `synced` recheck scope

The column starts `NULL`; the files already hold the tag. `POST /api/admin/pictures/recheck-exif`
(33 §8) gains `scope: "synced"`, and the admin Jobs tab a panel to start any scope.

- **Why `synced` is safe.** DB and file agree, so re-reading loses no edit; rows holding an unsynced
  edit are in other statuses and never touched. One exception, guarded in the finder: a `synced` row
  with **no `file_exif`** never observed its own file — a physical copy of a *received* picture is
  seeded with the recipient's overrides while its bytes hold the owner's original — so it is skipped.
- **No thumbnails.** `GenThumbnailConfig.metadata_only`: the claim hands out no thumbnail write URLs,
  and the worker already skips thumbnails without them. Decided **per row** (already thumbnailed),
  not per scope — an `extract_failed` row may have failed before its thumbnails and still needs them.
  A metadata-only pass reports no dimensions, so the stored decoded ones are kept.
- **Keyset cursor.** A re-read `synced` row lands back in `synced`, and a finished job frees its
  idempotency key, so a status-filtered sweep never drains. The sweep now walks `(ingested_at, id)`;
  this also stops the other scopes re-picking rows whose verdict did not change.

## 6. Documentation updates

- `doc/06_API_REFERENCE.md` — `gps_accuracy_m` on picture/edit/override/public payloads; the
  `synced` recheck scope.
- `doc/04_WORKER_ARCHITECTURE.md` — metadata-only `gen_thumbnail`.
- `doc/05_FRONTEND_ARCHITECTURE.md` — the accuracy control in the fix panels.
- `doc/features/30`, `33` — pointers here.

## 7. Work breakdown

- [x] Schema: `pictures.gps_accuracy_m` (migration `0021_gps_accuracy`); `FullExif` +
  `ExifField::GpsAccuracyM`; every picture read/write, received materialisation, physical copy,
  batch override, aggregate, public share.
- [x] Validation: range, one-way coupling.
- [x] Worker: read in both engines, write in both writers, rexiv2 clear-then-set.
- [x] Fix tools: suggested radius in `deriveGps`, `GpsAccuracyControl`, map radius, stale-accuracy
  removal; editor popover input; batch row.
- [x] Admin: `synced` scope, `metadata_only`, keyset cursor, `file_exif` guard, Jobs-tab panel.
- [x] Tests: worker round-trip through both writers + engine parity; validation; sweep cursor,
  guard and per-row thumbnails (`services_jobs.rs`); metadata-only claim end to end
  (`worker_contract.rs`).
- [ ] Rule field for the accuracy (§1, out of scope).
