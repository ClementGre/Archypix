# ExifTool as the Unified Metadata Engine (Sketch)

> **Status: sketch of a possible improvement — not a committed plan.** This document records the
> idea, the trade-offs, and a rough migration shape so it isn't lost. Nothing here is implemented;
> none of it should be treated as a decision. Revisit if/when the costs below are judged worth it.

## 1. Motivation

Metadata handling in the worker is currently split across two engines and three code paths:

- **Image read + write** — `rexiv2` (Rust → GExiv2 → Exiv2), an in-process native library:
  `imaging/exif.rs` (`extract_exif` / `write_exif_overrides`).
- **Video read** — `ffprobe` (subprocess): `imaging/video.rs` (`extract_video_metadata`).
- **Video write** — *nothing*. Exiv2/GExiv2 can't write video containers (and dropped video read in
  0.28), so video EXIF edits are DB-only (`exif_sync_status = unsupported`, no `edit_picture` job).
  See [`04_better_exif_support.md`](04_better_exif_support.md) and
  [`09_trash_and_exif_overrides.md`](09_trash_and_exif_overrides.md).

[ExifTool](https://exiftool.org/) could collapse all three into **one engine** that reads *and*
writes EXIF/XMP/IPTC for both images **and** video containers (MP4/MOV/MKV) — unlocking video
container write-back as a side effect, and deleting the bespoke video-tag parsing we hand-rolled
(ISO-6709, the Samsung `*.utc_offset` shift, the `com.android.version` make/model fallback), most of
which ExifTool composes natively.

## 2. What it would (and would not) replace

ExifTool is **metadata only**. It does not decode/resize images or grab video frames.

| Concern                                          | Today       | After (sketch)                |
|--------------------------------------------------|-------------|-------------------------------|
| Image EXIF read                                  | `rexiv2`    | **exiftool**                  |
| Image EXIF write                                 | `rexiv2`    | **exiftool**                  |
| Video metadata read                              | `ffprobe`   | **exiftool**                  |
| Video metadata write                             | — (DB-only) | **exiftool** (new capability) |
| Image thumbnails / blurhash / decoded dimensions | ImageMagick | ImageMagick (unchanged)       |
| Video frame-grab thumbnail                       | ffmpeg      | ffmpeg (unchanged)            |

Dependency delta: **drop** `gexiv2` + `exiv2` + `glib` (and the `gexiv2-sys` native link +
`PKG_CONFIG_PATH` setup in `flake.nix` / `worker/Dockerfile`); **add** `perl` + `exiftool`.
ImageMagick and ffmpeg stay.

## 3. Why it could be better

- **Coverage & correctness** — ExifTool is the most complete/correct metadata tool available: better
  RAW / maker-note / XMP / GPS / timezone handling than Exiv2, and full video-container support.
- **One model** — image + video, read + write, in one place, instead of the current three paths.
- **Build simplification** — a subprocess like ffmpeg; no compile-time native linking. One less
  fragile part of the build. (We already shell out to ffmpeg, so the "no subprocess in the hot path"
  property is already gone.)
- **Less custom code** — much of `imaging/video.rs`'s tag parsing becomes unnecessary.

## 4. The cost (the catch)

ExifTool is a **Perl process spawned per file**, vs. `rexiv2`'s in-process C call. Two implications:

1. **Per-file latency** — Perl startup is ~tens-to-hundreds of ms; today image extraction is
   in-process and fast, and every upload hits it. Mitigation: ExifTool's **`-stay_open True`** batch
   mode (one long-lived ExifTool process fed files over a pipe) removes nearly all startup cost, at
   the price of managing a persistent subprocess in the worker.
2. **Runtime weight** — adds a Perl interpreter to the worker image (tens of MB). Minor.

Video container write-back also inherits the costs noted in `04`/`09`: a metadata edit means a full
download + rewrite + re-upload of a potentially multi-GB original, and (because video has no
metadata-stripped `content_hash` — see [`11_physical_copy_and_dedup.md`](11_physical_copy_and_dedup.md))
the `file_hash` changes, so the dedup reconciler treats the edited file as new. For this reason video
write-back would likely stay **opt-in**, not the automatic per-edit behaviour images get.

## 5. Rough migration shape (if pursued)

1. Add `perl` + `exiftool` to `flake.nix` and the worker `Dockerfile`; drop `gexiv2`/`exiv2`/`glib`
   and the `gexiv2-sys`/`rexiv2` crate + `PKG_CONFIG_PATH` lines. Keep the startup-probe pattern
   (`imaging::video::tools_available`-style) for exiftool.
2. New `imaging/metadata.rs` (replacing `imaging/exif.rs`): `read_metadata(path) -> ExtractedExif`
   via `exiftool -json -n …` (one call works for images **and** video), mapping ExifTool tag names
   onto the existing `FullExif` / `CameraExif` shape. Fold the current video-specific normalisation
   (timezone, make/model fallback) into the mapping where ExifTool doesn't already do it.
3. `write_metadata(path, set, clear)` via exiftool for both images and video, preserving the
   file-untouched-on-failure invariant (write to a temp copy / verify, upload original last — the
   property `edit_picture` revert depends on).
4. `thumbnail.rs`: collapse the image/video metadata branch to a single `read_metadata` call; keep
   the engine split only for the **thumbnail** step (ImageMagick vs ffmpeg frame-grab).
5. Backend: let video into the EXIF-write path (stop forcing `exif_sync_status = unsupported`),
   gated behind an explicit opt-in for the file rewrite (§4). Keep DB-only as the default.
6. Optional: implement `-stay_open` batch mode if per-upload extraction latency proves to matter.

## 6. Recommendation

Attractive consolidation — it deletes code + native deps and unlocks video write-back — but the one
thing to weigh is extraction throughput (per-file Perl spawn vs in-process library). Worth doing if
per-upload latency is acceptable or the `-stay_open` plumbing is built; otherwise the in-process
`rexiv2` read path remains nicer for images. **No action until that call is made.**
