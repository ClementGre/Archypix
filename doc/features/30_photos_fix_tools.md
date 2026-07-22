# Photos fix tools

## 1. Overview & goals

Two guided modes for repairing the metadata most often missing from files — **GPS location**
and **capture date** — driven from the gallery, with highlight-in-context, per-field
auto-suggestions, and an explicit target→references selection flow for deriving values from
other pictures. Frontend-heavy: it reuses the existing EXIF write paths and the query
primitives in [29_query_proximity_and_missing_filter.md](29_query_proximity_and_missing_filter.md);
the only new backend surface is one nullable column and a possible batch-override extension.

Depends on: **29** (`has_gps`, presence filters, directed bracketing lookup), **04**
(single/batch write-through EXIF), **09/10** (received-picture override / propose-to-owner),
**14** (selection descriptor + batch dry-run).

## 2. Decisions (settled)

- **Highlight in context, not filter.** Fix mode keeps the whole grid and highlights the
  pictures missing the active field, so the user can compare against neighbours taken at the
  same time/place. (A standalone `missing` filter exists — feature 29 — but it is *not* how
  fix mode presents the grid.)
- **Explicit two-step selection (§7).** The user first selects the pictures to fix
  (**targets**), then optionally enters a distinct **reference-picking** phase to choose the
  pictures to derive from. No silent role inference — a picture is never turned into a target
  just because it lacks the field.
- **Auto-suggestion is grid-local by default (§5.2).** GPS interpolation reads the nearest
  GPS-bearing pictures *already loaded before/after in the grid*; only when the grid has no
  such neighbour (e.g. the user filtered to missing-GPS only) does it fall back to feature 29's
  **directed bracketing lookup** (29 §5 — `captured_before/after` + `gps=present` + `page_size=1`
  per side, *not* the proximity sort, which can't guarantee the bracket). No dedicated
  `gps-neighbors` endpoint.
- **Received pictures are in scope (§9)**, as both sources and targets — the flagship flow is
  giving a GPS-less camera's owned photos a location from a friend's received smartphone
  photos taken at the same moment. Received targets write via override / propose (09/10).
- **Capture-date sources (§6):** filename parser → source file mtime (new column) →
  `ingested_at` → derive-from-references. **No "other EXIF date field" source** — the worker
  already exhausts `DateTimeOriginal/DateTimeDigitized/DateTime` before `captured_at` goes
  NULL, so those fields are known-absent, and `exif_data` stores only `CameraExif`.
- **Source file date is suggestion-only.** Stored at ingest, never auto-applied to
  `captured_at` (unreliable — it is a modification time, not a capture time).
- **Bulk apply is preview-gated (§8)** with a per-row, nullable proposed value.

## 3. Fix mode UI

A **Fix** toggle (wrench) in the grid header, next to `TrashToggle`. Off by default; when on
it reveals a two-option sub-segment **GPS · Date** and swaps the right panel for the mode's
fix surface. Mode state lives in a URL param (`fix=gps|date`) via `useGalleryParams`, so it
is shareable/back-navigable like `trash`/`sort`.

## 4. Grid presentation

- **GPS mode → normal chronological sort, highlight only.** Missing-GPS pictures still have
  `captured_at`, so they already sit among their GPS-bearing neighbours — the exact context
  for interpolation. Highlight them (ring + a small `MapPinOff` badge) using
  `has_gps === false` (feature 29 §3).
- **Date mode → float missing to top.** Undated pictures have no `captured_at` and would sink
  to one end under any date sort. Add a fix-only ordering prefix
  `ORDER BY (captured_at IS NULL) DESC, <current sort>, filename, id` (a small extension to
  feature 29's sort SQL, gated on `fix=date`) so the broken ones surface at the top while the
  dated ones stay scrollable below as references. The **`filename, id` tiebreaker is
  load-bearing**: undated rows have no `captured_at` to order by, and run interpolation (§6)
  relies on them staying in a stable, filename-contiguous order across pages. Highlight via
  `captured_at == null`.

Highlight/detection runs on received rows too (§9).

## 5. GPS fix flow

`GpsFixPanel` in the right panel (extends the existing `GpsPickerPopover` /`MapView` point
mode).

### 5.1 Layout

A `MapView` showing up to three markers — the **before** anchor (thumb), the **after** anchor
(thumb), and the **proposed** pin (draggable) — with the interpolated coordinates, the
before/after time gap, and actions: **Apply**, **Pick references** (§7), **Copy from…**
(borrow one picture's exact coords), and **Next missing** (accept-and-advance).

### 5.2 Default suggestion (grid-local)

For a single target, scan the loaded, chronologically-sorted grid for the nearest
`has_gps` picture *before* and *after* the target's `captured_at`; highlight those two anchors
in the grid and compute the **time-weighted midpoint**
`p = p0 + (p1−p0)·(t−t0)/(t1−t0)` on lat/lng (and alt when both present). Most of the time the
immediate neighbours are correct and no fetch or reference-picking is needed.

### 5.3 Out-of-view fallback

When the grid has no GPS neighbour on a side (typically because the view is filtered to
missing-GPS only), fetch the two anchors with feature 29's **directed bracketing lookup**
(§5): one `captured_before` + `gps=present` query and one `captured_after` + `gps=present`
query, each `page_size=1`. These return the true nearest-before / nearest-after regardless of
how the other side clusters. The user can still open a **reference-picking view** (§7) to
override them.

### 5.4 Manual derivation

Via reference-picking (§7): the proposed pin becomes a derived average of the selected
references — **time-weighted interpolation only when exactly two references bracket the target
in time and the target is dated** (`t0 ≤ t ≤ t1`); with two same-side references, N > 2, or an
undated target it falls back to a plain **centroid** (no extrapolation past the ends). Manual
pin drag always overrides. **Copy-from-one** is just the single-reference case (average of one
= itself). Warn (badge) when the bracketing anchors are far apart in time — interpolation is
then a guess.

## 6. Capture-date fix flow

`DateFixPanel` = the existing `DateTimePickerPopover` plus a **suggestion chip row**, in
priority order:

1. **From filename** — a shared, tested parser (`lib/filenameDate.ts`) that tries **as many
   forms as possible** and returns a best guess. Offering an editable guess beats suggesting
   nothing — the user makes the final call, so the parser errs toward *a* result. It scans for
   a date-like substring across any separator (`-`, `_`, `.`, `:`, space, or none) and matches
   roughly in confidence order: `YYYYMMDD[_ -]HHMMSS`, `YYYY-MM-DD HH.MM.SS`, `YYYY.MM.DD`,
   bare `YYYYMMDD` / `YYYY-MM-DD`, camera/app prefixes (`IMG_`/`PXL_`/`VID_`/`Screenshot …` /
   WhatsApp `IMG-YYYYMMDD-WAxxxx`), textual months (`15 Aug 2023`, `Aug 15 2023`), day-first /
   month-first `DD-MM-YYYY` / `MM-DD-YYYY`, and 10/13-digit Unix epoch. Rules:
    - **Validity is a tiebreak, not a veto.** A candidate must resolve to a real date (reject
      month 13 / day 32 / `20231315`, year in `1900..=now+1`); an invalid reading is dropped so
      the *next* interpretation can win rather than suppressing the suggestion entirely.
    - **Ambiguity is resolved, then flagged.** When two components are both ≤ 12 (`05-08-2023`)
      the order is genuinely ambiguous: disambiguate by any component > 12 when present, else a
      default order, marked **low-confidence** (optionally offer the swapped reading as a second
      chip).
    - **Confidence is surfaced, never hidden.** Epoch and ambiguous matches are still offered,
      tagged low-confidence; in bulk (§8) low-confidence rows are **pre-flagged for review**,
      not excluded.
      The chip shows **which pattern matched** and its confidence; the value is always editable
      before apply, and null only when nothing plausible is found.
2. **Source file date** — `original_file_created_at` (§10), flagged *unreliable*.
3. **Uploaded** — `ingested_at`, always-available last resort.
4. **From references** — average of pictures the user recognises as same-time (§7).

Surface chips 1–3 in the **normal** date popover too (feature 04's inline editor) whenever the
capture-date field is empty — cheap, and it fixes pictures before fix mode is ever opened, and
optionally as a pre-fill in the `UploadDialog`.

**Run interpolation (optional):** select two **dated** references as the run's temporal ends
plus the **undated** targets between them; the targets are ordered by the date-mode grid's
`filename, id` tiebreaker (§4 — the only meaningful order they have) and their dates are evenly
spaced by position between the two ends. Requires the two references to actually bracket
(earlier end < later end); handy for scanned film rolls (`IMG_001…IMG_050`). Degenerate cases
(one end, un-orderable filenames) fall back to a single shared date.

## 7. The two-step selection model

The crux of the UX. Reuses the feature-14 gallery selection without silent role inference and
without a selection-persistence exception on normal browsing.

1. **Select targets.** Normal single/multi-select of the pictures to fix (the highlights help
   find them). The fix panel shows the auto-suggestion (§5.2 / §6) and can **Apply** straight
   away — the common single-picture path needs nothing more.
2. **Pick references.** Pressing **Pick references** *stashes* the target set and puts the
   gallery into a distinct **reference-picking phase** — a banner (*"Choosing references for N
   pictures"*), and a **fresh** selection. In this phase the user freely changes tag filters,
   scrolls, and selects the same-time / same-place pictures; the selection **persists across
   tag navigation** because that is the defining behaviour of the phase (normal selection is
   unchanged — still cleared on tag change).
3. **Preview & apply.** The panel previews the derived value (average / interpolation of the
   references) over the stashed targets; **Apply** writes; exiting the phase restores normal
   selection.

This makes the roles explicit and legible ("these get the average location of the references
you pick"), handles copy (1 ref), interpolate (2 refs), average (N refs), and bulk (N targets)
with one mechanism, and needs no exception to how normal selection clears.

## 8. Bulk apply

When targets > 1, **Apply** opens a preview list (styled like `BatchConfirmDialog`): one row
per target — `[thumb] filename → proposed value [editable] [✓ include]`. The proposed value is
**nullable**: a filename parse miss yields null → the row is skipped, and the user can blank
any row to opt it out even though it is selected. Two fill sources feed the rows — **per-target
filename dates / per-target interpolated GPS**, or a **shared derived value** (the reference
average). Confirm → one batch write (§11).

Generic over the field: bulk applies to GPS as well as dates.

## 9. Received pictures

In scope as sources and targets.

- **Sources** may be received (they only donate coords/dates; read-only).
- **Targets** may be received: apply routes per picture type — owned → write-through
  (`PATCH /pictures/exif`, feature 04); received → `POST /pictures/{id}/exif/override`
  (`mode:"local"` private override, or `mode:"propose"` where the share grants
  `allow_exif_edit`, feature 10). The fix panel exposes the local-vs-propose choice for
  received targets, matching `ExifInlineEditor`.
- **Bulk propose is supported by looping the per-picture path.** `propose_received_exif`
  (`services/pictures.rs`) already self-contains everything a proposal needs — it gates on
  *this* picture's share `allow_exif_edit`, resolves *this* owner, and routes
  same-backend-direct vs federation — all per picture. A bulk propose is therefore that call
  looped over the received subset; pictures whose share doesn't grant editing return `403` and
  are reported per-row (offered a local override instead), never a hard batch failure. So the
  received mode (local vs propose) is a **batch-level choice**; the backend handles the
  multi-share / multi-owner fan-out itself, one message per picture (feature 28 rate limits
  apply; a single batched propose message is a possible later optimisation).
- Highlight/detection uses `has_gps`/`captured_at` on received rows (feature 29 §3).

## 10. Schema: source file date

One nullable column, `pictures.original_file_created_at TIMESTAMP` (edit
`0001_initial_schema.up.sql` inline per the single-migration policy, then revert/run/`sqlx
prepare`). Populated at ingest, **never auto-applied** to `captured_at`:

- **Upload:** `BatchUploadFile` (`api/user/pictures.rs`) carries a per-file
  `original_file_created_at` from the browser's `File.lastModified` (epoch ms → naive local);
  persisted on create.
- **WebDAV PUT:** honour an `X-OC-Mtime` header (unix seconds, sent by Nextcloud/ownCloud-style
  clients) when present (`api/webdav.rs`); otherwise leave NULL — creation time is generally
  unavailable over WebDAV.
- Surfaced on `PictureListItem` + detail so the date chip (§6) can offer it.

**Timezone basis (advisory).** `File.lastModified` is a UTC instant rendered to the uploader's
**browser-local** wall clock; `X-OC-Mtime` is UTC. Both are stored naive and are only ever a
*suggestion* the user edits, so the mismatch with a photo's true local wall clock is acceptable
— it is never auto-applied to `captured_at` (§2).

## 11. Write paths (reuse)

- **Owned, single:** `POST /pictures/{id}/edit` (feature 04).
- **Owned, bulk:** `PATCH /pictures/exif` (feature 04) with the `set` shape.
- **Received:** `POST /pictures/{id}/exif/override` (09/10).
- **Mixed bulk:** a batch EXIF endpoint that splits the selection by route — owned →
  authoritative write-through (batch); received + `local` →
  `batch_apply_exif_received_local_selection` (already exists, selection-based); received +
  `propose` → loop `propose_received_exif` per picture (each self-gating/-routing, §9).
  Mirrors `batch_set_creator_selection`'s owned/received split (feature 14/26), extended with
  the propose branch; dry-run returns the per-route breakdown (owned / received-override /
  received-propose-eligible / grant-missing). This is the main backend logic addition beyond §10.

All of these already wake the pipeline and (for owned) re-announce to recipients — GPS/date
edits re-evaluate `gps_within_bbox` / `capture_year` / segments for free (feature 04 §3.1).

## 12. Edge cases

1. **Missing both date and GPS.** GPS interpolation needs a time anchor → the target sits
   undated at the top of the date-mode grid; fix the date first (select same-time references),
   then GPS. `GpsFixPanel` guards on null `captured_at` and says so.
2. **Only a before, or only an after** GPS anchor → no interpolation; offer copy-nearest or
   manual pin.
3. **`exif_sync_status = unsupported` MIME** → fix still writes the DB (source of truth for
   WebDAV/federation); the badge explains the file can't embed it.
4. **Far-apart anchors** → warn badge (§5.4).
5. **Timezone.** Filename / mtime / interpolation all produce naive local wall-clock,
   consistent with extraction; no conversion.
6. **Parser false positive** — a filename number that looks like a date. Mitigated by showing
   the matched pattern and the always-editable value + the bulk preview (§8). No EXIF undo
   history yet (separate roadmap item), so the preview gate matters.
7. **Reference-picking abandoned** (user navigates away / toggles fix off) → discard the
   stashed targets and restore normal selection.
8. **Received target without `allow_exif_edit`** → single-picture: propose is hidden, only
   local override offered. In a bulk **propose**, its per-picture call returns `403`; the
   preview reports that row and offers a local override for it instead of failing the batch.
9. **Trashed pictures.** Fix mode composes with the current `trash` filter but does **not**
   highlight or act on trashed rows (fixing metadata on a picture queued for purge is noise);
   detection is scoped to non-trashed. A user who wants to fix then restore does so explicitly.
10. **Hierarchy view.** When a hierarchy directory drives the grid (`useHierarchyBrowse`), the
    order is directory/name-based, not chronological, so **grid-local GPS interpolation (§5.2)
    is unavailable** — fall back to the directed bracketing lookup (§5.3) or reference-picking.
    `has_gps`/`captured_at` still flow through `browse` (feature 29) so highlighting works.
11. **Null-island `(0,0)` GPS.** Counts as *present* server-side (non-NULL), so it is not
    highlighted as missing. Offer an optional client heuristic to *also* flag `(0,0)` (and
    exact-integer 0 lat/lng) as suspect in GPS mode; off by default to avoid false positives on
    genuine equatorial/prime-meridian shots.
12. **Accept-and-advance across pages.** *Next missing* may need to page the grid forward; after
    a successful apply the fixed picture is optimistically de-highlighted (and drops from the
    fix set) via the existing `['pictures']` invalidation, so the cursor lands on the next
    still-missing one without a manual refresh.
13. **Reference-picking chrome.** During the reference phase (§7) the normal
    `SelectionActionBar` is replaced by the reference banner + *Use these references* / *Cancel*
    controls, so the batch trash/tag actions can't fire against the reference selection.

## 13. Documentation updates

- `doc/05_FRONTEND_ARCHITECTURE.md` — fix mode (`fix` param), `GpsFixPanel`/`DateFixPanel`,
  the two-step reference selection, the bulk preview, date chips in the normal editor +
  upload.
- `doc/04_WORKER_ARCHITECTURE.md` / `06_API_REFERENCE.md` — mixed-selection batch EXIF split;
  `original_file_created_at` on upload/WebDAV and list/detail.
- `doc/01_GENERAL_SPECIFICATIONS.md` — source file date is stored but never treated as capture
  date.

## 14. Work breakdown

- [ ] Schema: `pictures.original_file_created_at`; capture at upload (`File.lastModified`) +
  WebDAV (`X-OC-Mtime`); expose on list/detail; `sqlx prepare`.
- [ ] Fix mode: `fix` param + header toggle + GPS/Date sub-segment; highlight-in-context;
  date-mode missing-first sort prefix (extends feature 29 sort).
- [ ] `GpsFixPanel`: grid-local before/after interpolation, out-of-view directed-bracket fallback,
  copy-from-one, draggable pin, far-apart warn, next-missing.
- [ ] `DateFixPanel` + `lib/filenameDate.ts` parser (pattern table + tests); chips (filename /
  mtime / ingested); normal-editor + upload pre-fill; optional run interpolation.
- [ ] Two-step selection: target stash + reference-picking phase (persistent selection scoped
  to the phase) + preview.
- [ ] Bulk preview popup (per-row nullable value, include toggles) over both fields.
- [ ] Received targets: local/propose routing; mixed-selection batch EXIF split (owned
  write-through / received-local batch / received-propose per-picture loop) + per-route dry-run
  breakdown.
- [ ] Tests: filename parser matrix; grid-local + fallback interpolation; reference average
  (time-weighted 2 vs centroid N); bulk skip-on-null; received override/propose routing;
  mixed-selection split; pipeline re-eval on date/GPS edit.
- [ ] Docs (§13).
