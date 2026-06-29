# Feature 20 — Calendar Segmentation

## 1. Motivation

Today a `SegmentationTaggingService` is a list of `(name, date_range, assign_tag, parent_segment_id)`
rows, and its entire evaluator is "if `captured_at ∈ [start, end]` assign the tag". That is *exactly*
a feature-13 rule with a `captured_at` `date_range` leaf — segmentation adds **zero value over a
`RuleTaggingService`** and only duplicates it.

This feature repurposes the segmentation service into something a per-picture rule fundamentally
cannot be: a **partition operator** that auto-generates a set of date buckets and drops each picture
into **exactly one** of them, as subtags of a single root tag. The user describes the partition
declaratively (one bucket per year here, per season there, per decade before that, with custom
overrides) and the buckets are produced automatically and dynamically as pictures arrive — no
hand-authored range per bucket.

Everything in this feature is a **pure function of `captured_at`** (plus the static config), so it
slots into the existing per-picture evaluator with **no population pass**. Data-derived boundaries
(trip/event clustering by capture-time gaps or GPS proximity) are explicitly **out of scope** and
will be a separate future service type (see §13) — that is the line that keeps this feature simple.

---

## 2. Model

A segmentation service owns one **`SegmentationConfig`** (a JSONB object, §3). The config is an
ordered, **flat** list of **bands**. Each band:

- covers a half-open `captured_at` range `[from, to)` (either end open),
- carries a **template** that renders the picture's tag path *under the service root* (§4),
- optionally configures its template placeholders (`stride`, naming `format`) and a clock `offset`.

**Resolution is "first covering band wins"**: for a picture, the **first** band (array order = top
precedence) whose range contains `captured_at` produces its tag; no other band is consulted. This one
mechanism expresses both *subdivision* and *override*:

- A band whose template is `{year}` produces one bucket per year.
- A higher-precedence band over `[2015-08, 2016-08)` with a literal template `School_year_15_16`
  **replaces** the year tag for those pictures (it wins first).
- A higher-precedence band over `[2015, 2016)` with template `{year}.{month}` **subdivides** 2015
  deeper (its full path is `{year}`-prefixed).

There is **no recursive/child band structure** — bands are flat and each carries its *full* path
template. The editor *displays* a band B indented under band A when B's range ⊆ A's range and B's
template is a segment-prefix of A's (§12), but that is presentation only; the data model is a flat
list.

### 2.1 Core invariant

**A picture receives at most one segment tag from a given segmentation service** — the single leaf of
the winning band's rendered path. A multi-level template (`{year}.{month}`) stores only the deepest
label (`root.2024.August`); the `root.2024` ancestor is virtual (per the §1 tag model), not a second
segment. Bands may overlap *in the authored config* (that is how overrides work), but resolution
always collapses to one band, so the **effective** segmentation never overlaps.

---

## 3. `SegmentationConfig` schema

```jsonc
interface SegmentationConfig {
  version: 1;
  root_tag: string;            // ltree wire-form; every band's path hangs under this. Not under SharedToMe.
  hemisphere?: "north" | "south"; // default "north" — only affects {season} names
  catch_all: CatchAll | null;  // opt-in; null ⇒ a picture matching no band gets no segment tag
  bands: Band[];               // ordered; array index 0 = highest precedence
}

interface CatchAll {
  name: string;                // single ltree label ⇒ root_tag.<name> (e.g. "Unsorted")
  include_undated: boolean;    // true ⇒ pictures with captured_at = NULL land here too
}

interface Band {
  from: string | null;         // "YYYY-MM-DD" (NaiveDate); null = −∞. Range is half-open [from, to).
  to:   string | null;         // null = +∞
  enabled?: boolean;           // default true; false ⇒ the band is skipped in resolution (kept for re-enabling)
  template: string;            // placeholders + literals; "." separates subtag levels (§4)
  parts?: Record<string, PartConfig>; // keyed by placeholder name present in `template` (§5)
  offset?: Offset;             // clock shift applied before projecting placeholders (§6); default none
}

interface PartConfig {
  stride?: number;             // default 1 — group every N of this field (§5.2)
  format?: PartFormat;         // naming/formatting of the rendered value (§5.3)
}

interface PartFormat {
  numeric?: boolean;           // true = digits, false = name. Default is per-field (§4 table)
  pad?: number;                // numeric only: min digit width (zero-pad). 0/absent = no pad
  abbrev?: boolean;            // named only: short form (Aug) vs full (August). Default false
  case?: "lower" | "upper" | "pascal"; // applied to the rendered name. Default "pascal"
  bound?: "start" | "end" | "range";   // for a strided field: which end of the block to render. Default "start"
  range_sep?: string;          // bound = "range" only: separator between start and end. Default "_"
  inclusive_end?: boolean;     // bound = "end" | "range": false (default) ⇒ next-block boundary; true ⇒ last value in block
}

interface Offset {             // any subset; usually a single component. Subtracted from captured_at (§6).
  months?: number;
  days?: number;
  hours?: number;
  minutes?: number;
}
```

---

## 4. Template grammar

A template renders a picture's tag path **relative to `root_tag`**. It is a sequence of literal text
and `{placeholder}` references, with `.` as the **subtag-level separator**:

- `{year}` → one level: `2024`
- `{year}.{month}` → two levels: `2024.August`
- `Q{quarter}_{year}` → one level: `Q3_2024`
- `School_year_15_16` (no placeholder) → one fixed level: a single bucket over the band's whole range

Rules:

- Each dot-separated segment of the template must contain **at least one placeholder or one
  alphanumeric literal character** (no empty levels).
- After substitution, each level is **sanitized to a valid ltree label** `[A-Za-z0-9_]`: every run of
  disallowed characters becomes a single `_`, and leading/trailing `_` are trimmed (`"Summer 2024"` →
  `Summer_2024`). Literal `.` cannot appear inside a level — dots are always structural.
- The full stored tag is `root_tag` + `.` + the rendered path. Only the deepest label is stored
  (§2.1).

### 4.1 Placeholder catalog

Each placeholder is a pure projection of `captured_at` (after `offset`, §6). All are valid in any
band.

| Placeholder | Value             | Numeric default | Named forms (abbrev / full)           |
|-------------|-------------------|-----------------|---------------------------------------|
| `year`      | calendar year     | numeric         | —                                     |
| `iso_year`  | ISO week-year     | numeric         | —                                     |
| `quarter`   | 1–4               | numeric         | —                                     |
| `season`    | 1–4 (hemisphere)  | **name**        | Win / Winter, Spr / Spring, …         |
| `month`     | 1–12              | numeric         | Aug / August                          |
| `week`      | ISO week 1–53     | numeric         | —                                     |
| `day`       | day-of-month 1–31 | numeric         | —                                     |
| `weekday`   | 1–7 (Mon = 1)     | **name**        | Mon / Monday                          |
| `daypart`   | 1–4               | **name**        | Night / Morning / Afternoon / Evening |

- `daypart` boundaries: Night `[00:00,06:00)`, Morning `[06:00,12:00)`, Afternoon `[12:00,18:00)`,
  Evening `[18:00,24:00)`.
- `season` (northern, default): Winter = Dec–Feb, Spring = Mar–May, Summer = Jun–Aug, Autumn =
  Sep–Nov. `hemisphere: "south"` shifts by two seasons. Names are English only (no i18n).
- Numeric-only fields (`year`, `iso_year`, `quarter`, `month`, `week`, `day`) reject `numeric: false`
  / `abbrev` (validation, §9). Named fields still accept `numeric: true` to render the underlying number.
- `week` is the ISO week; pair it with **`iso_year`**, not `year`, since ISO week 1 of a year can fall
  in the previous calendar year (`{iso_year}.W{week}` → `2024.W52` correctly, where `{year}.{week}`
  could read `2025.W52`). `{year}` remains the plain calendar year for every non-week template.
- `mm / mmm / mmmm / YYYY`-style forms all come from `PartFormat`:

| Want          | `parts.<field>.format`                     |
|---------------|--------------------------------------------|
| `YYYY` 2024   | year:  `{ pad: 4 }`                        |
| `m` 8         | month: `{}`                                |
| `mm` 08       | month: `{ pad: 2 }`                        |
| `mmm` Aug     | month: `{ numeric: false, abbrev: true }`  |
| `mmmm` August | month: `{ numeric: false }`                |
| `august`      | month: `{ numeric: false, case: "lower" }` |

---

## 5. Placeholder configuration (`parts`)

`parts` is keyed by placeholder name; an entry tunes how that placeholder is bucketed and rendered.
Placeholders without an entry use defaults.

### 5.1 Grouping key

The **set of placeholders present** is the partition key. `{month}` alone yields 12 buckets ever (all
Augusts together, regardless of year); `{year}.{month}` yields month-within-year. There is no `unit`
field — the template fully specifies granularity, depth, grouping, and names.

### 5.2 `stride`

`stride` (default 1) groups the field's values into blocks of N, starting from the field's natural
origin (`0` for `year`; `1` for the cyclic fields), **within its parent cycle**:

- `year: { stride: 10 }`, template `{year}s` → decade blocks named `1990s`, `2000s` (block start ×10).
- `year: { stride: 5 }` → 5-year blocks aligned to multiples of 5 (2000, 2005, …).
- `day: { stride: 5 }` → day-of-month blocks `1–5, 6–10, …, 26–31` **reset each month** (the last
  block is short). Continuous cross-month rolling blocks are intentionally **not** supported here —
  that is linear/clustering territory (§13).
- `month: { stride: 2 }` → bimonthly within a year.

A strided placeholder renders the **block's start value** by default; `format.bound` selects another
end (§5.4).

### 5.3 `format`

Per the `PartFormat` shape (§3) and the forms table (§4.1). `case`/`abbrev`/`numeric` shape the label;
`pad` zero-pads numeric output.

### 5.4 Strided block naming (`bound`)

For a strided field the placeholder can render the block start, end, or the start–end pair:

- `bound: "start"` (default) → `2020`
- `bound: "end"` → `2025` (next-block boundary) or `2024` with `inclusive_end: true`
- `bound: "range"` → `2020_2025` (or `2020_2024` with `inclusive_end: true`); `range_sep` sets the
  separator.

Example: `{year}` with `{ stride: 5, format: { bound: "range", range_sep: "_" } }` → `2020_2025`.
There is no template modifier for the end value (the placeholder appears once); interleaving arbitrary
text between two independent endpoints is out of scope (that is trip-style naming, §13).

---

## 6. `offset` — boundary shifting

`offset` is a calendar duration **subtracted from `captured_at` before any placeholder is projected**,
so the period boundary moves *later* by that amount. It is a property of the **band** (not a
placeholder), because it must apply uniformly to every level of the path — otherwise a 4 am day
boundary would roll `{day}` to the previous day while leaving `{year}`/`{month}` on the next, producing
an inconsistent path.

| Want                                | `offset`        |
|-------------------------------------|-----------------|
| Day ends at 4 am (photographic day) | `{ hours: 4 }`  |
| Fiscal year starts in April         | `{ months: 3 }` |
| Week starts Sunday (vs ISO Monday)  | `{ days: 1 }`   |

`offset` affects **projection and rendering** (a 2 am Jan-1 capture correctly renders under Dec 31),
but **not band membership** `[from, to)` — bands are matched against the raw `captured_at`. This is a
deliberate choice: `from`/`to` are real calendar dates the user authored, and a sub-day shift at a
year-granularity boundary is negligible. Projection order within a band: `shifted = captured_at −
offset` → project field → apply `stride` block → render via `format`.

`offset` is **not** a stride phase-alignment knob (which would bucket-shift *without* changing the
rendered year). Strided blocks always align to the field's natural origin (§5.2); a custom stride
phase is out of scope for v1.

---

## 7. Resolution algorithm

For a picture with `captured_at = t` (a `NaiveDateTime`):

1. If `t` is `NULL`: assign `root_tag.<catch_all.name>` iff `catch_all` is set and
   `include_undated`; otherwise no segment tag.
2. Walk `bands` in order, skipping any with `enabled: false`; select the **first** remaining band
   whose `[from, to)` contains `t`. (Open ends are unbounded.)
3. If none match: assign `root_tag.<catch_all.name>` iff `catch_all` is set; otherwise no tag.
4. For the winning band, render its template: for each placeholder, project `t − offset`, apply
   `stride`, format the value; join levels with `.`; sanitize each level (§4).
5. The stored tag is `root_tag` + `.` + the rendered path (a single `source = segment` leaf).

This yields **at most one** tag per picture, structurally enforcing §2.1. It is a pure function of
`t` and the config — no other picture is consulted, and an empty bucket costs nothing (a bucket's tag
exists only once a picture lands in it, so empty-bucket suppression needs no config).

---

## 8. Worked example

```jsonc
{
  "version": 1,
  "root_tag": "Photos.Travel",
  "catch_all": { "name": "Undated", "include_undated": true },
  "bands": [
    { "from": "2015-08-01", "to": "2016-08-01", "template": "School_year_15_16" },
    { "from": "2020-01-01", "to": null, "template": "{season}_{year}",
      "parts": { "season": { "format": { "abbrev": false } } } },
    { "from": "2015-01-01", "to": "2016-01-01", "template": "{year}.{month}",
      "parts": { "month": { "format": { "numeric": false } } } },
    { "from": "2000-01-01", "to": "2020-01-01", "template": "{year}" },
    { "from": null, "to": "2000-01-01", "template": "{year}s",
      "parts": { "year": { "stride": 10 } } }
  ]
}
```

| `captured_at` | Winning band         | Tag                               |
|---------------|----------------------|-----------------------------------|
| 1994-06-10    | decade band          | `Photos.Travel.1990s`             |
| 2018-03-02    | 2000–2020 year band  | `Photos.Travel.2018`              |
| 2015-03-20    | 2015 subdivision     | `Photos.Travel.2015.March`        |
| 2015-10-04    | school-year override | `Photos.Travel.School_year_15_16` |
| 2021-07-15    | season band          | `Photos.Travel.Summer_2021`       |
| *(none)*      | catch-all            | `Photos.Travel.Undated`           |

(The 2015 subdivision band sits above the year band, so 2015-03 hits it before the year band; the
school-year override sits above the subdivision, so Aug 2015–Jul 2016 captures resolve to it first.)

---

## 9. Validation (server-side, on create/update)

- `root_tag` and `catch_all.name` are valid ltree labels/paths, not under `SharedToMe`.
- For every band: `from < to` when both are set; each `{placeholder}` is in the catalog (§4.1); every
  `parts` key appears in `template`; each dot-segment of `template` is non-empty (§4).
- `PartFormat`: `numeric: false` / `abbrev` only on named fields; `pad`/`stride ≥ 1`/`offset`
  components are non-negative; `range_sep`/`inclusive_end` only meaningful with a strided field.
- **Lint (warn, not reject)** — surfaced in the editor, not fatal:
    - two bands at the same precedence rank with overlapping ranges that aren't an override/subdivision
      relationship (ambiguous);
    - a band fully shadowed by a higher-precedence band with an equal-or-broader range (dead band);
    - an uncovered timeline gap when `catch_all` is `null`.
- Errors identify the offending band index / placeholder (e.g.
  `"bands[2].parts.month: field 'month' has no named form for abbrev"`).

---

## 10. Storage: unified service config (refactor)

This feature does the storage refactor at the same time. **Fold every service type's config into a
single `config jsonb` column on `tagging_services` and drop all three per-type child tables**
(`shared_tag_mapping_services`, `rule_tagging_services`, `segmentation_tagging_services`). A service's
config is read and written as one whole object — the evaluator never touches an individual rule/segment
row — so the relational split bought nothing and forced three joins per evaluation. This also matches
how feature-13 predicates and hierarchy configs already live as JSONB.

```sql
ALTER TABLE public.tagging_services ADD COLUMN config jsonb NOT NULL DEFAULT '{}'::jsonb;
-- after the data migration (§11):
DROP TABLE public.shared_tag_mapping_services;
DROP TABLE public.rule_tagging_services;
DROP TABLE public.segmentation_tagging_services;
```

`service_type` stays (evaluation dispatch, ordering, admin filters). `config` holds the type-specific
payload:

```jsonc
// service_type = "rule"
{ "rules": [ { "id": uuid, "predicate": <feature-13 tree>, "assign_tag": string } ] }
// array order = display order; the old `position` column is dropped

// service_type = "shared_tag_mapping"  — one service PER incoming share (§10.1)
{ "incoming_share_id": uuid, "assign_tags": string[] }

// service_type = "segmentation"
<SegmentationConfig>   // §3
```

Per-item `id`s on rules are kept so the granular endpoints and the frontend can address one rule
without rewriting the array.

### 10.1 `shared_tag_mapping` becomes one service per share

Rules and segments are self-contained blobs — inlining them is pure win. Mappings are **not** fully
self-contained: a mapping references an `incoming_shares` row, and two operations query mappings
**across services and users**, keyed on `incoming_share_id` / `is_broken`, never by service —
the counter-example to "a service config is only ever read whole":

- revocation flags every mapping of a revoked share broken
  (`UPDATE shared_tag_mapping_services SET is_broken = true WHERE incoming_share_id = $1`,
  `repository/tagging.rs`);
- the admin consistency check counts broken mappings instance-wide
  (`SELECT COUNT(*) … WHERE is_broken = true`, `repository/admin.rs`).

Resolution — restructure the type so its cross-cutting key is first-class: **one
`shared_tag_mapping` service per incoming share**, its config a scalar `incoming_share_id` plus the
list of tags to assign. This matches what the data already implies — `find_or_create` today reuses a
single per-owner service holding N rule rows, and `incoming_shares.local_mapping_service_id` FKs the
*rule* row, not the service (`services/shares/shareback.rs`). One-service-per-share makes that link
point at a real service and turns `incoming_share_id` into an indexable per-service scalar.

With that shape:

- **Brokenness is derived, not stored.** A mapping service is broken iff its `incoming_share_id` has
  no *active* `incoming_shares` row (revoked / tombstoned / absent). Drop the `is_broken` column and
  the revocation→tagging-config `UPDATE` entirely — revocation no longer touches tagging state.
- **The admin count becomes a join:**
  `SELECT count(*) FROM tagging_services ts JOIN incoming_shares i ON (ts.config->>'incoming_share_id')::uuid = i.id WHERE ts.service_type = 'shared_tag_mapping' AND i.status <> 'active'`.
- Evaluation already gates on the picture's *active* incoming-share ids, so a broken mapping yields no
  tags regardless — the old `!is_broken` filter (`domain/pipeline.rs`) was redundant and is removed.
- `incoming_shares.local_mapping_service_id` now references the **service** id.

An expression index `CREATE INDEX … ON tagging_services ((config->>'incoming_share_id')) WHERE
service_type = 'shared_tag_mapping'` supports the join/derive queries — this is the "index on the
broken field" made concrete (we index the share id, since brokenness is derived from it).

### 10.2 One uniform config-editing path

Every service type is edited the **same way**: the whole type-specific config is replaced via
`PUT /tagging-services/{id}/config` (and supplied at create via `POST /tagging-services`'s `config`
field). There are **no** granular per-rule / per-segment / per-mapping sub-resources, and no separate
reorder endpoint — the array (rules) or band order in the submitted config *is* the stored order, so
reordering / adding / removing a rule is just a `PUT` with the new array. This mirrors how
segmentation and hierarchy configs already work, and keeps the three types consistent. The server
validates + normalizes the config in one place (`ServiceConfig::parse`): rule predicates (feature 13),
assigned tags (non-protected ltree), and segmentation bands (§9); rules submitted without an `id` get
one assigned.

---

## 11. Migration of existing data

A single migration backfills `config` for all three types, then drops the child tables. Dev data is
minimal (as with feature 13).

1. **`rule` services** — set `config = { "rules": [ {id, predicate, assign_tag} … ] }` from each
   service's `rule_tagging_services` rows, ordered by the old `position` (array order replaces it).
2. **`shared_tag_mapping` services** — **split** each existing (per-owner) mapping service into **one
   service per mapping row**: for each `shared_tag_mapping_services` row, create a
   `shared_tag_mapping` service owned by the same user with
   `config = { "incoming_share_id": …, "assign_tags": [<assign_tag>] }`, copying the source service's
   `requires`/`excludes`/`enabled`. Repoint each `incoming_shares.local_mapping_service_id` from the
   old rule-row id to the new service id. Delete the now-empty original service(s). (`is_broken` is
   dropped — it is derived now, §10.1.)
3. **`segmentation` services** — old segmentation rows are semantically plain `captured_at` range
   rules, so convert each **old segmentation service into a `rule` service**: build
   `config = { "rules": [ { "field": "captured_at", "date_range": { from: lower(date_range), to: upper(date_range) } } → assign_tag … ] }`
   from its segments (`parent_segment_id` is dropped — it only nested the tag string, already encoded
   in `assign_tag`) and flip `service_type` from `segmentation` to `rule`.

Then `DROP TABLE` all three child tables. No existing service starts as a new-model calendar
segmentation — the new model is opt-in for services created after the migration.

> Converting old segments to fixed (no-placeholder) bands is rejected: they carry arbitrary
> `assign_tag`s that need not share a common root, which the band model's single `root_tag` cannot
> represent without inventing one. Converting to rules preserves exact behaviour.

---

## 12. Evaluation, API, and frontend

### 12.1 Domain evaluation

The domain layer is organized for consistency across the three service types:

- `domain/predicate.rs` — the feature-13 rule predicate engine (`Predicate`, `Field`, `Condition`,
  parsing), split out of the (previously oversized) `domain/pipeline.rs`.
- `domain/segmentation.rs` — `SegmentationConfig` parse/validate/resolve (template parsing,
  placeholder projection, `stride`/`offset`/`format` rendering).
- `domain/pipeline.rs` — now just the `PipelineInput` picture projection.
- `domain/tagging.rs` — the service model and the **single** evaluation hub: `ServiceConfig` (a
  `Rule | Segmentation | SharedTagMapping` enum) with `parse` (validate + normalize raw JSON),
  `to_value` (storage-ready JSON), `source`, and one `evaluate(input, incoming_share_ids) ->
  ServiceResult` dispatch. Gating is `TaggingService::should_run(current_tags)`. Calendar
  segmentation resolves to **zero or one** tag (§7); shared-tag-mapping drops the `!is_broken` filter
  (redundant — derived, §10.1).

The pipeline (`infra/routine/pipeline/evaluation.rs`) parses each service's `config` column once into
a `ServiceConfig` (replacing the three `*RuleRepository::list_for_services` joins with one read) and
calls `config.evaluate(...)` per picture. Dirty detection, per-source reconciliation
(`source = segment`/`rule`/`share_mapping`), ordering, and gating are unchanged.

### 12.2 API

The three service types are edited **uniformly** — there are no per-type sub-resource endpoints:

- `ServiceDetailResponse` arms: rule → `rules[]`; segmentation → `config: SegmentationConfig`;
  shared-mapping → `{ incoming_share_id, assign_tags, is_broken }` with **`is_broken` computed**
  (derived from the share's status, §10.1).
- **`POST /tagging-services`** takes `{ service_type, name?, requires?, excludes?, config? }` for any
  type. `config` is the type-specific object (validated per type; a mapping's `incoming_share_id`
  must be the caller's). Omitted `config` falls back to that type's empty config (mapping has none —
  it must be supplied). Returns the `ServiceDetailResponse`.
- **`PUT /tagging-services/{id}/config`** `{ config }` replaces the whole config for any type
  (validated identically; rules get ids assigned). This is the **only** config-editing path — no
  add/edit/delete/reorder sub-resources for rules, segments, or mappings. Array/band order in the
  submitted config is the stored order.
- `incoming_shares.local_mapping_service_id` now references the mapping **service**.
- Removed: `POST/DELETE /tagging-services/{id}/segments[...]`, `…/rules[...]`, `…/rules/reorder`,
  `…/mappings[...]`, `…/mapping`, `…/segmentation`.

Update `06_API_REFERENCE.md §6.8` and `01_GENERAL_SPECIFICATIONS.md §3.2/§3.4` accordingly.

### 12.3 Frontend

`SegmentEditor` becomes a band-list editor: ordered (drag-reorder = precedence) list of band cards,
each with `from`/`to` (`DateRangePicker`, date mode, open-ended allowed), a `template` field, a
per-placeholder `parts` panel (stride + format controls), and an `offset` control. A live preview
renders the bucket names a band produces over a sample range, and the editor displays a band indented
under another when it is range-contained and template-prefixed (the §2 child-display inference). The
editor surfaces the §9 lints inline. `catch_all`, `root_tag`, and `hemisphere` are service-level
fields. See `05_FRONTEND_ARCHITECTURE.md §7 (tagging/)`.

---

## 13. Out of scope / future: clustering service

Anything that needs to look at **other pictures** is deliberately excluded and reserved for a future
**clustering** service type:

- **Event/trip detection** — start a new segment when the capture-time gap to the previous picture
  exceeds a threshold, optionally also breaking on GPS distance.
- **Min-pictures-per-bucket merging** — "don't create a segment for 2 stray photos" requires counts,
  i.e. a population pass.
- **Continuous cross-boundary N-day blocks** and **trip-style start–end naming** with two independent
  endpoints.

Keeping calendar segmentation a pure `captured_at` function is what makes that boundary crisp: the
moment a boundary depends on the population, it belongs to the clustering service, not here.

---

## 14. What this does NOT change

- The `requires`/`excludes` service-level gate (unchanged; `root_tag` is independent of `requires`).
- The pipeline execution model: dirty detection, wake/debounce, evaluation order
  (`SharedTagMapping` first, then Rule/Segmentation by `position`), per-source reconciliation,
  service enable/disable/delete (`promote_tags`) lifecycle.
- The **behaviour** of `RuleTaggingService` and `SharedTagMappingService` (predicate matching, share→tag
  mapping). Their **storage** moves into `tagging_services.config` and `shared_tag_mapping` becomes
  one-service-per-share (§10), but what they assign is unchanged.
- The tag storage model (per-source rows, virtual ancestors, `source = segment`).
