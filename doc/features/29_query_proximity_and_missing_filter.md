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
  missing the sort field sort last; `SortOrder` is ignored (always nearest-first).
- **Geo distance is approximate, sort-only** — a cheap equirectangular metric, no
  PostGIS/`earthdistance` dependency, never surfaced as a number.
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

Time:

```sql
ORDER BY (captured_at IS NULL),                       -- undated last
         abs(extract(epoch FROM (captured_at -
$near_time
)
)
)
ASC
```

Geo (equirectangular, sort-only):

```sql
ORDER BY (gps_latitude IS NULL OR gps_longitude IS NULL),   -- ungeotagged last
         (gps_latitude -
$near_lat
)
^
2
+
(
(
gps_longitude
-
$near_lng
)
*
cos
(
radians
(
$near_lat
)
)
)
^
2
ASC
```

Append a stable tiebreaker (`id`) so pagination is deterministic when many rows share a
distance.

## 7. Selection threading

`PictureFilter` (`services/selection.rs`) gains `gps`/`capture_date`/`missing_any` so a
feature-14 selection descriptor can express "every picture missing GPS" for a batch action
(dry-run `count_selection` flows through). Proximity **sorts** do not apply to a selection (a
set, not an ordered page).

## 8. Frontend

- `useGalleryParams`: add `gps`/`capture_date` (`?…=present|missing`, default `any`),
  `missing_any`, and the two proximity sorts (`sort=time_near|geo_near` with `near_time` /
  `near_lat,near_lng`).
- A small **issues filter** control near `TrashToggle`: *All · Missing GPS · Missing date ·
  Any issue* → writes `gps=missing` / `capture_date=missing` / `missing_any=true`. Standalone
  value: users find problem pictures without a tagging service.
- Proximity sorts are mostly programmatic — a picture context action **"Find nearby in time /
  place"** sets `sort` + `near_*` and clears tag filters. Minimal UI here; the fix tools (30)
  drive the metadata-repair path.

## 9. Edge cases

1. **Proximity sort without its reference param** → 400.
2. **`missing_any` combined with a per-field presence** → 400 (§4).
3. **Received rows** report `has_gps` / capture-date presence from their last-announced
   columns; announcement staleness is the normal federation caveat, not a bug here.
4. **Antimeridian / poles** distort the equirectangular geo metric — acceptable for a *sort*
   (worst case a slightly wrong ordering of far-apart candidates); documented, not fixed.
5. **Null-island GPS `(0,0)`** counts as *present* (it is non-NULL). Some devices write it for
   a failed fix; treating it as missing is a heuristic left to the fix tools (30 §12), not this
   filter.

## 10. Documentation updates

- `doc/06_API_REFERENCE.md` — list/browse/selection params gain `gps`, `capture_date`,
  `missing_any`, `sort=time_near|geo_near`, `near_time`, `near_lat`, `near_lng`;
  `PictureListItem.has_gps`.
- `doc/03_BACKEND_ARCHITECTURE.md` — presence + proximity arms in `push_filters` / sort SQL.
- `doc/05_FRONTEND_ARCHITECTURE.md` — `useGalleryParams` params; the issues-filter control.

## 11. Work breakdown

- [ ] `PresenceFilter` + `missing_any`; `push_filters` arms + `missing_any` mutual-exclusion
  400; thread through `PictureListParams`, hierarchy `browse`, `PictureFilter`; `sqlx prepare`.
- [ ] `has_gps` derived field on `PictureListItem` (owned + received).
- [ ] `PictureSortField::{TimeNear, GeoNear}` + `near_*` params + required-param 400 + stable
  tiebreaker; sort SQL.
- [ ] Frontend `useGalleryParams` params + issues-filter control + "Find nearby" context action.
- [ ] Tests: presence arms (present/missing per field) + AND composition + `missing_any` OR +
  mutual-exclusion 400; directed bracketing lookup (§5) returns the right single row per side;
  proximity ordering + field-missing-last + stable tiebreak; selection `count_selection` with a
  presence filter.
- [ ] Docs (§10).
