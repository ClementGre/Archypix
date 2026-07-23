# Query presence filters & proximity sorts

## 1. Overview & goals

General gallery-query primitives, useful on their own and the substrate for the fix tools
([30_photos_fix_tools.md](30_photos_fix_tools.md)). Additions to the picture list / hierarchy
`browse` / selection query, none of which change *what* pictures are visible except the
presence filters:

1. **Presence filters** — per-field tri-state (`any | present | missing`) over GPS and capture
   date, so the gallery can surface *and* exclude pictures by metadata completeness. The
   negation ("present") is as important as "missing": it is how the fix tools isolate
   interpolation anchors.
2. **`has_gps` list field** — a derived boolean so the client can highlight and scan without a
   round-trip (capture-date presence is already known from `captured_at`).
3. **Time- / geo-proximity sorts** — order a query by `|captured_at − ref|` or distance to a
   `lat,lng` ("what else did I shoot around then / near here"). A **browse** ordering, distinct
   from the directed bracketing lookups the fix tools use (§5).

Read/query only — no schema change beyond a derived list field, no write path.

## 2. Decisions (settled)

- **Presence is a per-field tri-state filter, AND-composed** — not a single "missing" enum.
  This gives the negation for free and lets the fix tools express the *interpolatable set*
  ("missing GPS **but** dated") as `?gps=missing&capture_date=present`.
- **"Any issue" is a separate OR arm.** `(gps missing OR capture_date missing)` cannot be
  expressed by AND-composed per-field filters, so it is its own `missing_any=true` param,
  mutually exclusive with the per-field ones.
- **Bracketing ≠ proximity.** Finding the nearest picture *before* and *after* a reference
  (the interpolation-anchor need) is done with the **existing** `captured_before`/
  `captured_after` bounds + a presence filter + `page_size=1` per side — two directed,
  correct queries. The **proximity sort** is a browse ordering (nearest on both sides
  interleaved by absolute delta) and is *not* used for anchors, because a side with many
  closer neighbours can push the other side's nearest off the first page.
- **Proximity sort needs a reference point** (`near_time`, or `near_lat`/`near_lng`); rows
  missing the sort field are **excluded** from the result (a proximity sort by X is meaningless for a
  row with no X — it would only ever trail the page as noise); `SortOrder` is ignored (always
  nearest-first). `near_time` is a **naive** instant (no offset), compared against the naive
  `captured_at` column — the client passes a picture's `captured_at` string straight through.
- **Geo distance is haversine, sort-only** — order by the haversine central-angle term `a`
  (monotonic with true great-circle distance, so exact *for an ordering* while skipping the final
  `asin`/`R` scaling). No PostGIS/`earthdistance` dependency and no index: the sort runs on the
  per-user, gps-present candidate set (already covered by `idx_pictures_gps`) as a top-N heapsort,
  which is sub-10ms at 10–50k rows — a KNN GiST index only pays off on huge, lightly-filtered
  scopes and composes poorly with the tag/trash filters. The distance is never surfaced as a number.
- No new endpoints — extend the existing query surfaces.

## 3. `has_gps` on the list item

`PictureListItem` (`services/pictures.rs`) gains `has_gps: bool` =
`gps_latitude IS NOT NULL AND gps_longitude IS NOT NULL`, for owned **and** received rows
(received GPS lives in the promoted columns refreshed by `create_received`, feature 04 §10.2).
This drives highlight-in-context and the client's grid-local anchor scan (30 §4, §5.2).

## 4. Presence filters

```rust
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceFilter { #[default] Any, Present, Missing }
```

- `gps: PresenceFilter` — `Present` → `gps_latitude IS NOT NULL AND gps_longitude IS NOT NULL`;
  `Missing` → the negation.
- `capture_date: PresenceFilter` — over `captured_at`.
- `missing_any: bool` — the OR convenience (§2), `(gps IS NULL OR captured_at IS NULL)`;
  rejected (400) if combined with a non-`any` `gps`/`capture_date`.
- Add all three to `PictureListParams` (`services/pictures.rs`), hierarchy `browse`
  (`api/user/hierarchies.rs`), and `PictureFilter` (`services/selection.rs`) so a feature-14
  selection can target "everything missing GPS". Wire `?gps=present|missing`,
  `?capture_date=present|missing`, `?missing_any=true`.
- Emit in `push_filters` (the helper that emits the trash arms), AND-composed with every other
  arm. Composes with `untagged`, tag filters, `captured_*`, etc.
- **Indexing:** the presence predicates are residuals on top of the per-user/`owner_id` scan
  and are cheap; add partial indexes (`WHERE gps_latitude IS NULL`, `WHERE captured_at IS NULL`)
  only if the "any issue" sweep over a large library measures slow. Optional.

## 5. Directed bracketing lookup (no new surface)

The nearest dated, GPS-bearing picture on each side of a reference instant — the fix tools'
anchor need (30 §5.2–5.3) — is expressed with **existing** params plus §4:

- **before:** `captured_before=<ref>&gps=present&sort=captured_at&order=desc&page_size=1`
- **after:** `captured_after=<ref>&gps=present&sort=captured_at&order=asc&page_size=1`

(`captured_before`/`captured_after` are `DateTime<Utc>`; the caller passes the target's naive
`captured_at` interpreted as UTC, matching `selection.rs`'s existing `naive_utc()` convention.)
No proximity sort, no bespoke endpoint.

## 6. Proximity sorts (browse)

Extend `PictureSortField` with `TimeNear` / `GeoNear`, each requiring its reference param
(400 if absent). For the general "browse what's nearby" UX — interleaves both sides by
distance; a consumer wanting a strict bracket uses §5 instead.

**Rows missing the sort field are excluded, not trailed.** A proximity sort by X is meaningless for a
row with no X, so `push_filters` appends `captured_at IS NOT NULL` (time) / `gps_lat IS NOT NULL AND
gps_lng IS NOT NULL` (geo) under the respective sort — the `total` count and the page agree, and the
ORDER BY drops the old "nulls last" leading key.

Time (`near_time` is a naive instant):

```sql
ORDER BY abs(extract(epoch FROM (captured_at -
$near_time
)
)
)
ASC,
id
ASC
```

Geo (haversine central-angle term, sort-only):

```sql
ORDER BY sin(radians(gps_lat -
$near_lat
) /
2
)
^
2
+
cos
(
radians
(
$near_lat
)
)
*
cos
(
radians
(
gps_lat
)
)
*
sin
(
radians
(
gps_lng
-
$near_lng
) /
2
)
^
2
ASC,
id
ASC
```

Ordering by the haversine `a` term is monotonic with the great-circle distance, so it is exact for
a sort without the final `asin`/earth-radius scaling; `sin²(Δlng/2)` folds the antimeridian
correctly. The stable `id` tiebreaker keeps pagination deterministic when many rows share a distance.

**Distance surfaced on the list item (geo only).** Under a `geo_near` sort, `PictureListItem` gains
`distance_m: Option<f64>` — the great-circle distance in metres from the reference, computed **in
Rust** over the returned page (`services/pictures.rs`, the same haversine as the ORDER BY, so the
badge and the ordering never disagree). `None` off a geo sort and for ungeotagged rows;
`skip_serializing_if` keeps it off the wire otherwise. It is geo-specific by necessity: the client
already knows `near_time` + each row's `captured_at` (so a *time* delta is client-derivable), but the
list item never exposes raw GPS coordinates, so the geo distance can only come from the server.

## 7. Selection threading

`PictureFilter` (`services/selection.rs`) gains `gps`/`capture_date`/`missing_any` so a
feature-14 selection descriptor can express "every picture missing GPS" for a batch action
(dry-run `count_selection` flows through). Proximity **sorts** do not apply to a selection (a
set, not an ordered page).

## 8. Frontend

- `useGalleryParams`: add `gps`/`capture_date` (`?…=present|missing`, default `any`),
  `missing_any`, and the two proximity sorts (`sort=time_near|geo_near` with `near_time` /
  `near_lat,near_lng`).
- An **`IssuesFilter`** dropdown next to `TrashToggle`: GPS and capture-date are each an independent
  **three-state** control (*Any · Present · Missing*) so the user can hunt for problem pictures
  *and* isolate the good anchors (the `present` invert), plus an *Any issue* toggle for the
  `missing_any` OR (mutually exclusive with the per-field states). On phones the grid-header
  breadcrumb and these buttons stack vertically so neither is squeezed.
- Proximity sorts are programmatic — `SelectionPanel`'s overflow (`⋯`) menu carries **Find nearby in
  time / place** alongside Download; each sets `sort` + `near_*` and clears tag, hierarchy **and**
  presence filters (the last matters because a geo sort already excludes ungeotagged rows).
  `FilterControls`' Sort menu surfaces the active proximity mode with a one-click clear.
- Under a `geo_near` sort each tile shows a **distance badge** from `distance_m` (formatted `m`/`km`);
  under `time_near` the tile shows a **time-delta badge** computed client-side from `captured_at` −
  `near_time` (auto unit s/min/h/d/mo/y — no backend field needed, both timestamps naive).

## 9. Edge cases

1. **Proximity sort without its reference param** → 400.
2. **`missing_any` combined with a per-field presence** → 400 (§4).
3. **Received rows** report `has_gps` / capture-date presence from their last-announced
   columns; announcement staleness is the normal federation caveat, not a bug here.
4. **Antimeridian / poles** — the haversine `a` term folds longitude wraparound correctly
   (`sin²(Δlng/2)`) and stays well-behaved at the poles, so the geo ordering is exact there (this is
   the reason for haversine over the cheaper equirectangular metric).
5. **Null-island GPS `(0,0)`** counts as *present* (it is non-NULL). Some devices write it for
   a failed fix; treating it as missing is a heuristic left to the fix tools (30 §12), not this
   filter.

## 10. Documentation updates

- `doc/06_API_REFERENCE.md` — list/browse/selection params gain `gps`, `capture_date`,
  `missing_any`, `sort=time_near|geo_near`, `near_time`, `near_lat`, `near_lng`;
  `PictureListItem.has_gps` + `distance_m` (geo-sort only).
- `doc/03_BACKEND_ARCHITECTURE.md` — presence + proximity arms in `push_filters` / sort SQL.
- `doc/05_FRONTEND_ARCHITECTURE.md` — `useGalleryParams` params; the issues-filter control.

## 11. Work breakdown

- [x] `PresenceFilter` + `missing_any`; `push_filters` arms + `missing_any` mutual-exclusion
  400 (`PictureListFilter::validate`); thread through `PictureListParams`, hierarchy `browse`,
  `PictureFilter` (via `ScopeParams`). (Query-builder SQL — no `.sqlx` macro to prepare.)
- [x] `has_gps` derived field on `PictureListItem` (owned + received); geo-sort-only `distance_m`.
- [x] `PictureSortField::{TimeNear, GeoNear}` + `near_*` params + required-param 400 + stable
  `id` tiebreaker; `push_order_by` sort SQL (haversine geo ordering).
- [x] Frontend `useGalleryParams` params + `IssuesFilter` control + `SelectionPanel` "Nearby in
  time/place" actions + `FilterControls` proximity indicator/clear + `PhotoCard` distance badge.
- [x] Tests (`back/tests/presence_and_proximity.rs`): presence arms (present/missing per field) +
  AND composition + `missing_any` OR + mutual-exclusion 400 + proximity required-param 400; directed
  bracketing lookup (§5) returns the right single row per side; proximity ordering +
  field-missing-last + stable tiebreak + antimeridian; selection `count_selection` with a presence
  filter.
- [x] Docs (§10): 06 API reference, 03 backend architecture, 05 frontend (below).
