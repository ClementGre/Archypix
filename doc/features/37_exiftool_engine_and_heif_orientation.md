# Feature 37: ExifTool as the only metadata engine, and HEIF orientation

## 1. Overview & goals

Rotating a HEIC in Archypix changed its EXIF `Orientation`, but the file itself stayed unrotated
everywhere else: HEIF viewers (ImageIO — Preview, Quick Look, Photos — and libheif) take the
orientation from the primary item's `irot`/`imir` properties and ignore EXIF `Orientation` entirely,
even when no `irot` exists. The read-back then re-read EXIF and reported the file `synced`. The read
side had the mirror bug: a HEIF whose `irot` and EXIF disagree displayed differently in Archypix
than anywhere else (hidden in practice because Apple writes both).

Fixing it on top of two engines (rexiv2 + ExifTool, feature 33) meant a third code path, so the
engine layer collapses to ExifTool alone, plus a small HEIF box writer for orientation.

Also fixed: mirrored orientations (2/4/5/7) were stored and rotated correctly but displayed without
the mirror.

Out of scope: a flip action (the front would need one table; HEIF writes already support it), video
container writes (feature 19), widening `MIME_TYPES_EXIF`.

## 2. Decisions (settled)

- **ExifTool is the only metadata engine.** Settles feature 19 §6 and supersedes 33 §2.1–2.2. The
  blocker of 33 §3.4 — one stay-open child behind a mutex — is gone with a process pool (§3.1). The
  `file_exif` backfill concern: before rexiv2 was deleted the parity corpus proved ExifTool maps
  every normalization point identically; the corpus now pins rexiv2's values as expectations. Where
  the two would differ, ExifTool's own write → read round trip wins.
- **HEIF orientation lives in the container.** For a HEIF file (top-level `meta` with a `pict`
  handler — sniffed, not taken from the MIME) the orientation is read from and written to the
  primary item's `irot`/`imir`. EXIF `Orientation` is still written to the same value (Apple's
  convention) but never read.
- **Our own box writer.** No Linux tool creates the properties: ExifTool edits an existing `irot`
  (and `imir` since 13.43) but never adds one, libheif cannot rewrite a file it has read, ImageIO is
  macOS-only. `metadata/bmff.rs` rewrites `ipco`/`ipma` and shifts `iloc`. Debian's ExifTool stays.
- **Apple's encoding, byte for byte.** A target orientation always writes an `irot` (angle 0 for
  orientation 1, as ImageIO does), so `Some(1)` and `None` round-trip distinctly; `None` removes both
  properties. Mirrored orientations use ImageIO's choice of axis (§3.4).
- **Thumbnails follow the primary.** The transform is applied to the primary item and every item
  that is its `thmb`, as ImageIO does; Quick Look may draw the embedded thumbnail.

## 3. Worker

`imaging/metadata.rs` — `read(path)` / `write_target(path, target)`, no MIME parameter:
`metadata/exiftool.rs` handles the tags, then `metadata/bmff.rs` the HEIF orientation (a no-op for
any other file). Both blocking.

### 3.1 Process pool

Idle stay-open processes wait in a `Mutex<Vec<ExifTool>>`; a call takes one or spawns one, and puts
it back unless the call hit a tool fault (dead child, broken pipe, poisoned lock), so the pool grows
to peak concurrency (`max_concurrent_jobs`) and a crashed child is replaced. The crate reports a child
that died mid-call as a process error (`"Process terminated unexpectedly."`); it is now classified
`ToolUnavailable` (retriable) instead of falling through to a file verdict.

### 3.2 Read

ExifTool's mapping is unchanged (33 §3.2–3.3); the dispatch and the rexiv2 → ExifTool fallback
(33 §3.1) are gone, so an ExifTool format error is the file verdict. For HEIF the orientation is then
replaced by the container's: `None` without `irot`/`imir`, else their composition.

**Precision is what the file holds.** `FullExif::normalize` (archypix-common) rounds the 35 mm focal
length to an integer (`FocalLengthIn35mmFormat` is an integer tag), the f-number and the GPS accuracy
to 2 decimals (f/1.78 is a real aperture; one decimal read it as 1.8). The reader applies it, and so
does edit validation — an edit is stored as the file will hold it, so the read-back and a re-read
return the stored value.

### 3.3 Write

ExifTool writes the target in one pass — every `Some` set, every `None` deleted (31 §3.3) — then, for
HEIF, the box writer sets the orientation on ExifTool's output. Text values are stripped of control
characters: stay-open arguments are newline-separated, so a newline in a camera model (from the API,
a federated proposal, or a crafted file round-tripping through the DB) would inject ExifTool options.

**One capture date per file.** The untagged date names also update their same-named XMP copies
(`xmp:CreateDate`, `exif:DateTimeOriginal`); every other copy — `photoshop:DateCreated`,
`exif:DateTimeDigitized`, `tiff:DateTime` in XMP, and the IPTC IIM `DateCreated`/`TimeCreated`/
`DigitalCreation*` — is deleted on every write. A stale IIM date is not harmless: once a tool rewrote
the IIM block without refreshing its digest (rexiv2 did), MWG readers such as ImageIO treat it as the
newer value and show it over EXIF. Rewriting rather than deleting them was rejected: IIM times need a
zone, which Archypix does not model.

### 3.4 The box writer

- **Orientation algebra.** An EXIF orientation is `(k, f)`: display = rotate `k` quarter turns
  clockwise after an optional left-right flip `f`. `irot a` (counter-clockwise) adds `4 − a` to `k`;
  `imir` flips left-right (axis 1) or top-bottom (axis 0), mapping `k` to `−k` or `2 − k`. Properties
  apply in association order (MIAF: `clap`, `irot`, `imir`). Reads compose any sequence; writes emit
  ImageIO's pair: 1 → `irot 0`, 3 → `irot 2`, 6 → `irot 3`, 8 → `irot 1`, 2 → `irot 0` + LR,
  4 → `irot 0` + TB, 5 → `irot 1` + TB, 7 → `irot 1` + LR.
- **Rewrite.** Each target item drops its `irot`/`imir` associations and appends the new ones
  (essential). An identical property box is reused; `irot`/`imir` boxes nothing references any more
  are dropped and the indices renumbered (7- or 15-bit as needed). All `ipma` boxes merge into one.
- **Offsets.** The `meta` size change is applied to every `iloc` location (construction method 0,
  this file) at or past the old `meta` end — the base offset when it lies there, else each extent.
  Refused as a write failure: a `moov` after `meta` (its chunk offsets would move too), item data
  stored inside `meta`, and an offset that no longer fits its field.
- **Errors.** A HEIF the reader cannot parse is a file verdict (`unsupported_file`); a layout the
  writer refuses is `write_failed`.

## 4. Frontend

`orientationTransform` returns a `mirror` flag; mirrored orientations render `rotate(θ) scaleX(-1)`
(5 → 270°, 7 → 90°, since the flip now applies first).

## 5. Documentation updates

- `doc/04_WORKER_ARCHITECTURE.md` — module layout, EXIF sections.
- `doc/06_API_REFERENCE.md` — edit rounding.
- `doc/02_INFRASTRUCTURE_DESIGN.md`, `worker/README.md` — engine mentions.
- `doc/features/19`, `33`, `36` — pointers here.
- `AGENTS.md` feature index, `doc/99_ROADMAP.md`.

## 6. Work breakdown

- [x] `imaging/exif.rs` → `imaging/metadata.rs` + `metadata/exiftool.rs` + `metadata/bmff.rs`.
- [x] ExifTool pool; dead-child classification; rexiv2, gexiv2 and the dispatch/fallback deleted
  (Cargo, Dockerfile, flake).
- [x] One-pass target writer with control-character stripping; stale date copies deleted.
- [x] `FullExif::normalize`: 2-decimal f-number read, edits rounded to what the file holds.
- [x] HEIF orientation read + write (`irot`/`imir`, thumbnails, `iloc` shift).
- [x] Front: mirrored display.
- [x] Tests: corpus pinned to rexiv2's reads; write → read round trip (JPEG + HEIC); no stale date
  copy; ImageIO reference files 1–8; box rewrite (payload preserved, thumbnails, no growth on
  repeated writes); validation rounding.
- [ ] Flip action (§1, out of scope).
