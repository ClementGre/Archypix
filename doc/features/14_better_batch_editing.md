# Feature 14 — Better Batch Editing

## 1. Motivation

The frontend offers very limited multi-picture viewing and editing. Today a multi-selection can only
batch-add tags and batch trash/restore (one request per picture), and the right panel shows nothing
about what the selected pictures have in common.

Two problems compound:

- **No aggregate view.** There is no way to see the tags/EXIF/metadata common to a selection, which
  tags are on *some* but not all, or how mixed EXIF values are distributed.
- **Selection doesn't scale.** Multi-select is a `string[]` of ids materialised client-side. A
  `Ctrl+A` over a filtered view can only select the pictures already loaded into the infinite-scroll
  grid, and a large selection would ship thousands of ids on every batch call.

This spec introduces a **selection descriptor** (a query + add/remove deltas) reused by every batch
endpoint, a **type-aware aggregation endpoint** for the multi-select panel, and the batch write
surface (tags, EXIF with owner-suggestion, trash/restore) — including a deferred EXIF-job model so a
batch edit over thousands of pictures is a single set-based write.

---

## 2. The selection descriptor

Every batch endpoint accepts the same `PictureSelection`:

```ts
interface PictureSelection {
    query: PictureFilter | null;   // the homogenized filter (see §3); null ⇒ pure explicit set
    include_ids: string[];         // pictures explicitly added
    exclude_ids: string[];         // pictures subtracted from the query result
}
```

**Effective set** = `(resolve(query) ∪ include_ids) \ exclude_ids`, always scoped server-side to the
caller's own holdings.

Two modes fall out of one model:

- **Explicit mode** — `query: null`, `include_ids` = the clicked pictures. The degenerate
  single-picture case is `query == null && include_ids.len() == 1 && exclude_ids.is_empty()`; it is
  handled by the same code path as any batch.
- **Select-all mode** — `query` = the current view's filter, `exclude_ids` = the un-checked
  pictures. `Ctrl+A` adopts the query and clears the deltas; un-checking pushes to `exclude_ids`;
  checking a picture outside the query pushes to `include_ids`.

### 2.1 Consistency

`resolve(query)` runs at **apply time**, with no point-in-time pinning. `Ctrl+A` means "everything
this query matches now" — including pictures not yet loaded into the grid and pictures ingested since
the user selected. This is intentional: the mandatory confirmation popup always shows the *resolved*
count (§6.1), so the count is honest even when the matched set drifts.

### 2.2 Shift-range selection

A shift-click range is materialised into `include_ids` client-side. To shift-click a far target the
infinite-scroll grid must already have loaded every picture up to it, so those ids are in memory and
bounded by what was rendered — a dedicated range/cursor primitive is unnecessary. (A future
"select by date/sort range without loading" feature would express the range as keyset cursor bounds
on the query — `(sort_key, id) BETWEEN cursorA AND cursorB` — folded into `PictureFilter`; out of
scope here.)

### 2.3 Selection store & URL

The frontend `selection` store grows from `selected: string[]` to the descriptor. Because the
descriptor is small (a filter plus two short id lists) it is URL-encodable, consistent with the
existing "view state belongs in the URL" principle, so a large multi-edit selection is shareable and
back/forward-friendly.

---

## 3. Homogenized picture filter

`GET /pictures` and `GET /hierarchies/{id}/browse` already resolve to the same predicate internally
(`browse` resolves a directory path into a "most-specific node wins" predicate and reuses the picture
list machinery). This feature unifies the **filter type** so the selection descriptor speaks one
language over the flat gallery and a hierarchy directory:

```ts
type PictureFilter =
    | {
    kind: "flat"; /* existing GET /pictures params: tag/include_tags/exclude_tags/match,
                       scope (owned_only/shared_with_me), untagged, include_deleted,
                       captured_after/before, sort, order */
}
    | { kind: "hierarchy"; hierarchy_id: string; path: string; /* + the same flat scope/date params */ };
```

The `hierarchy` form is resolved server-side to a predicate (`predicate_for_path`) and AND-ed with
any flat scope/date params. The hierarchy's writability is a write-side concern and does not affect
read/selection resolution.

Merging the two **HTTP routes** (`browse` into `/pictures`) is a later cosmetic follow-up; the
high-value, low-risk part is unifying the filter *type*, which is what makes the selection model and
all batch endpoints work identically across views.

---

## 4. Batch read — aggregation endpoint

### `POST /api/authenticated/pictures/aggregate`

Computes aggregate information over a `PictureSelection` server-side (a GROUP BY / conditional
aggregate), so a select-all of 10k pictures never has to be materialised or downloaded.

**Request:**

```ts
{
    selection: PictureSelection;
    sections ? : Array<"summary" | "tags" | "exif">;   // default ["summary"]
    tag_provenance ? : boolean;                          // default false; only meaningful with "tags"
}
```

`sections` keeps the sidebar cheap: the panel fetches `summary` immediately and requests `tags` /
`exif` only when those foldable sections are expanded (§6.2). The **tag** section is the heaviest
(ltree ancestor expansion, §4.2) and benefits most from this laziness.

**Response `200`:**

```ts
{
    // summary — always returned, all from the pictures row (zero joins)
    count: number;
    owned_count: number;
    received_count: number;
    total_file_size: number;
    trashed_count: number;            // deleted_at IS NOT NULL
    owner_deleting_count: number;     // received: owner_deleted_at IS NOT NULL
    thumbnail_pending_count: number;  // thumbnails_generated_at IS NULL
    duplicate_count: number;          // pictures sharing a file_hash with another in the selection
    owners: Array<{ username: string; instance: string; count: number }>;
    exif_sync: Record<ExifSyncStatus, number>;   // histogram incl. the new pending_job_creation (§5)

    // tags — only when "tags" requested
    tags ? : Array<{
        path: string;          // ancestor-expanded, folded to distinct paths
        count: number;         // count == total ⇒ on all; < total ⇒ on some
        manual_count: number;  // pictures holding a *manual* row under this path (drives removability)
        sources?: Array<{ source: TagSource; count: number }>;  // only when tag_provenance=true
    }>;

    // exif — only when "exif" requested; per-field, type-aware (see §4.3)
    exif ? : Record<string, FieldAggregate>;
}
```

### 4.1 Summary fields

All summary fields read straight off the `pictures` row — no joins. `duplicate_count`
(`GROUP BY file_hash HAVING count(*) > 1`) pre-wires the upcoming dedup work. The `exif_sync`
histogram is the convergence signal for in-flight batch edits (§6.3).

`suggestable` (how many received pictures may be proposed to their owner) is **not** computed here —
it requires a join through to `incoming_shares.allow_exif_edit` that is too costly to run on every
sidebar refresh. It is a dry-run output instead (§6.1).

### 4.2 Tag aggregation

Counts are **ancestor-inclusive**: a picture stored as `/Photos/Travel/Spain` contributes to
`/Photos`, `/Photos/Travel`, and `/Photos/Travel/Spain`. `count == total` ⇒ the tag is on every
selected picture (solid chip); `count < total` ⇒ on some (indeterminate chip).

`manual_count` is a single `FILTER (WHERE source = 'manual')` aggregate and reflects the **removal
semantics**: batch remove deletes `WHERE tag_path <@ <path> AND source = 'manual'`
(`TagRepository::batch_remove`), so removing `/Photos/Travel` drops a stored manual
`/Photos/Travel/Spain` too — but a `rule`/`segment`/`share_mapping`-sourced row is untouched and
reappears on the next pipeline run. The UI therefore shows the ✕ / remove affordance only where
`manual_count > 0`, and the dry-run reports "removes from M of N".

Full per-source provenance (`sources`) is opt-in (`tag_provenance=true`) and runs a heavier
path×source query, mirroring the single-picture provenance toggle.

### 4.3 Type-aware field aggregates

```ts
type FieldAggregate =
    | {
    type: "distinct"; common: unknown | null; distinct: Array<{ value: unknown; count: number }>;
    distinct_overflow: number; null_count: number
}                 // string / enum fields
    | { type: "numeric"; min: number | null; max: number | null; avg: number | null; null_count: number }
    | { type: "date"; min: string | null; max: string | null; avg: string | null; null_count: number }
    | {
    type: "gps"; bbox: { lat_min; lat_max; lng_min; lng_max } | null;
    centroid: { lat; lng } | null; null_count: number
};
```

- **string/enum** (camera_brand/model, mime_type, …) — distinct values capped at **10** +
  `distinct_overflow` count. The frontend shows the first few inline and the rest in a tooltip
  (still capped). `common` is set when the distinct set collapses to one value.
- **numeric** (iso_speed, f_number, focal_length_mm, exposure_time_num, exposure_time_den, file_size, width, height,
  gps_alt, orientation, …) — `min`/`max`/`avg`. The frontend merges `exposure_time_num`/`_den` back into one `n/d s`
  rational row (mirroring the single-picture editor).
- **date** (captured_at, ingested_at, updated_at) — `min`/`max` (range) + `avg` instant.
- **gps** — `bbox` (exact, cheap) + `centroid`; the frontend draws a rectangle or enclosing circle
  via the existing `MapView` bbox/circle modes. (Naive lat/lng averaging is wrong across the
  antimeridian — irrelevant for a normal library.)
- **`null_count`** on every nullable field ("8 of 10 have GPS"), which also powers
  "set the field on the ones missing it" actions.

All of the above is single-pass conditional aggregation — trivial at 10k rows for Postgres.

---

## 5. Deferred EXIF jobs

A batch EXIF edit cannot enumerate-then-create one `edit_picture` job per picture: the selection is a
query, not a list, and a 10k-picture edit must not create 10k jobs synchronously in the request.

Mirroring the tagging pipeline's dirty-then-drain pattern:

1. The batch write is a **single set-based** `UPDATE … WHERE <resolved predicate>` that applies the
   `set`/`clear` to the DB and stamps a new EXIF sync state
   `exif_sync_status = 'pending_job_creation'`. No enumeration; scales to the whole selection.
2. A drain task (the `ExifDrain` `Routine`, feature 17 — interval sweep + immediate `trigger`) selects
   rows in that state with no active `edit_picture` job, creates jobs in batches, and flips them to
   `pending`. The existing `stuck_exif_pending` consistency check already half-describes this drain.

**Owned vs received partition.** The fast column write-through applies to owned pictures. Received
pictures take the `local_exif_overrides` JSONB **merge** path (`exif_data || override`, still
set-based) and never produce a file job (§6 for the propose path). The endpoint partitions the
resolved set accordingly.

**No synchronous `job_id`.** Because jobs are created by the drain, the batch response cannot return
per-picture job ids (and polling thousands of jobs is a non-starter). Convergence is tracked through
the `exif_sync` histogram (§4.1, §6.3): `pending_job_creation → pending → synced` (or `errored` /
`unsupported`). The single-picture `POST /pictures/{id}/edit` may keep returning a `job_id` for
backward compatibility, but the status histogram is the uniform signal.

---

## 6. Batch write surface

All batch writes accept a `PictureSelection`, resolve it inside the transaction, support
`dry_run: true`, and are gated by a mandatory confirmation popup on the frontend.

| Endpoint                                   | Status  | Change                                                                                                                 |
|--------------------------------------------|---------|------------------------------------------------------------------------------------------------------------------------|
| `PATCH /api/authenticated/tags`            | exists  | accept `selection`; tristate add/remove (§6.4)                                                                         |
| `PATCH /api/authenticated/pictures/exif`   | exists  | accept `selection`; owner-mode (§6.1); deferred jobs (§5); `set`/`empty`/`clear` (`empty` = override-to-null, 10 §6.3) |
| `POST /api/authenticated/pictures/trash`   | **new** | batch soft-delete (replaces per-picture loop)                                                                          |
| `POST /api/authenticated/pictures/restore` | **new** | batch restore                                                                                                          |

### 6.1 Dry-run & owner modes

The expensive "what exactly will happen" computation runs **once, when the confirmation popup opens**,
via the same endpoint with `dry_run: true` — never on sidebar refresh. Using the same
endpoint/resolution guarantees the preview cannot diverge from the apply.

The popup opens with a loader, runs the dry-run, then shows the affected breakdown, pre-selects the
mode default, and enables Confirm. For EXIF the breakdown distinguishes the two batch modes:

- **Edit locally** — owned → write-through; received → local override.
- **Edit & suggest to owner where allowed** — owned → write-through (caller is the owner);
  received with `incoming_shares.allow_exif_edit` → propose to owner; received without the grant →
  fall back to local override.

The dry-run resolves the owner-suggestability join (received tag → `incoming_shares.allow_exif_edit`)
and returns e.g. `{ affected: 10, edited: 5, suggested: 3, local_override: 2 }`. When the selection is
owned-only, both mode options are hidden (plain edit). Dry-run response shape:

```ts
{
    affected: number;
    // EXIF batch only:
    edited ? : number;
    suggested ? : number;
    local_override ? : number;
    unsupported ? : number;
    // tags batch only:
    added ? : number;
    removed ? : number;   // removed counts pictures with a manual row under the path
    // trash/restore: just `affected`
}
```

### 6.2 Aggregate fan-out control

Manual toggles change `count` by exactly ±1 on a known-membership picture, so the displayed count is
updated **optimistically client-side with zero requests**. The heavier `tags`/`exif` aggregation is
**debounced** (~300–400 ms after the selection settles; descriptor in the TanStack Query key with
`keepPreviousData`), so e.g. un-checking ten pictures after a `Ctrl+A` issues one aggregate request,
not ten. Combined with per-section laziness (§4), the Tags GROUP BY only runs if its section is open.

### 6.3 Convergence display

After a batch EXIF apply the sidebar shows a progress bar derived from the `exif_sync` histogram
(synced / queued / pending / errored), re-fetched at a backoff rate scaled to selection size.

### 6.4 Tristate tags

The multi-select Tags section shows each tag as checked (on all), indeterminate (on some), or absent.
Toggling adds or removes across the selection. Removal only affects `manual` rows (§4.2), so the
remove affordance and its dry-run count reflect `manual_count`, not `count`.

### 6.5 Idempotency

Large batch writes accept an idempotency key (reusing the jobs idempotency pattern) so a retried
big trash/edit does not double-apply.

---

## 7. Frontend

- **`selection` store** holds the descriptor; `Ctrl+A`/"Select all" adopts the current `PictureFilter`
  instead of enumerating loaded ids.
- **Floating action bar** is shown on **desktop too** whenever more than one picture is selected (not
  just mobile): count, Select-all, **Invert** (`swap(include, exclude)` relative to the query), Clear,
  Batch actions.
- **Multi-select right panel** reuses the single-picture section layout (Summary / Tags / EXIF), fed
  by `/aggregate` with per-section lazy fetch. EXIF rows show the common value or a "Mixed"
  affordance opening the distinct-values / min-max-avg detail; GPS renders the bbox/centroid on
  `MapView`.
- **Confirmation popups** are mandatory for every batch action and host the dry-run loader/result.

---

## 8. Out of scope

- **Undo of batch operations** — folded into the separate "EXIF edit history" v1.0 item.
- **Async bulk-operation jobs** — the deferred-job drain (§5) covers EXIF; other writes are set-based.
  A confirmation threshold + internal chunking guards very large destructive batches; a full bulk-job
  framework is deferred until a real need appears.
- **Keyset cursor-range selection** (§2.2) — only if a "select by range without loading" feature lands.
- **Merging the `browse` HTTP route into `/pictures`** — cosmetic follow-up; the filter *type* is
  unified now, the routes can converge later.
