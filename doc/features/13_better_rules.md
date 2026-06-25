# Feature 13 — Better Rule Tagging Predicates

## 1. Motivation

The current `RuleTaggingService` stores predicates as free-text strings
(`gps_within_bbox(…)`, `capture_year(…)`, etc.). This has two problems:

- **No composition.** A single rule row maps one flat predicate to one tag. There is no way to
  express "assign tag X only when condition A AND condition B are both true."
- **Limited coverage.** Only 4 predicates exist, none covering camera EXIF fields, file metadata,
  or ownership.

This spec replaces the text predicate with a structured JSONB predicate tree that supports
arbitrary AND/OR/NOT composition and a typed field+condition model covering all queryable picture
attributes.

---

## 2. Predicate model

### 2.1 Tree structure

A predicate is a recursive JSON value. Three logical nodes and two spatial nodes sit at the top
level alongside field conditions:

```jsonc
// Logical
{"and": [<predicate>, ...]}          // all children must match
{"or":  [<predicate>, ...]}          // at least one child must match
{"not": <predicate>}                 // inverts child

// Spatial predicate (multi-field — the only exceptions to the single-field model)
{"gps_bbox":   {"lat_min": 45.0, "lat_max": 46.0, "lon_min": 4.0, "lon_max": 5.0}}
{"gps_radius": {"lat": 48.86, "lng": 2.35, "km": 50.0}}

// Field predicate (the main leaf form — see §2.2 and §2.3)
{"field": "<field_name>", "<condition_key>": <value>, ...}
```

Empty `and`/`or` arrays are valid: `{"and": []}` always matches, `{"or": []}` never matches.

### 2.2 Fields

Every leaf predicate names exactly one `field`. The full set of available fields and their base
types are:

| `field` name      | Base type | Source column / derivation                              |
|-------------------|-----------|---------------------------------------------------------|
| `captured_at`     | date      | `pictures.captured_at`                                  |
| `gps_lat`         | float     | `pictures.gps_lat`                                      |
| `gps_lng`         | float     | `pictures.gps_lng`                                      |
| `gps_alt`         | int       | `pictures.gps_alt` (metres)                             |
| `iso_speed`       | int       | `exif_data.iso_speed`                                   |
| `f_number`        | float     | `exif_data.f_number`                                    |
| `focal_length_mm` | float     | `exif_data.focal_length_mm`                             |
| `exposure_time`   | float     | `exif_data.exposure_time_num / exposure_time_den` (sec) |
| `orientation`     | int       | `pictures.orientation` (EXIF tag: 1/3/6/8/…)            |
| `camera_brand`    | str       | `exif_data.camera_brand`                                |
| `camera_model`    | str       | `exif_data.camera_model`                                |
| `filename`        | str       | `pictures.filename`                                     |
| `mime_type`       | str       | `pictures.mime_type`                                    |
| `file_size`       | int       | `pictures.file_size` (bytes)                            |
| `width`           | int       | `pictures.width` (pixels)                               |
| `height`          | int       | `pictures.height` (pixels)                              |
| `is_owned`        | bool      | `pictures.remote_picture_id IS NULL`                    |

### 2.3 Conditions per base type

The `condition_key` (and optional extra keys) depend on the field's base type. Runtime validation
at creation time rejects conditions that don't match the field's declared type.

**int and float** — the same condition keys apply to both; float fields accept decimal values:

```jsonc
{"field": "iso_speed",  "eq":  400}
{"field": "iso_speed",  "min": 100}
{"field": "iso_speed",  "max": 800}
{"field": "iso_speed",  "min": 100, "max": 800}   // half-bounded ranges are fine
{"field": "f_number",   "max": 2.8}
{"field": "exposure_time", "min": 0.5}             // slower than 1/2 s
```

**str:**

```jsonc
{"field": "mime_type",     "eq":       "image/heic"}                       // exact, case-sensitive
{"field": "camera_brand",  "eq":       "fujifilm", "ignore_case": true}    // exact, case-insensitive
{"field": "camera_brand",  "contains": "Canon"}                            // substring, case-sensitive
{"field": "camera_brand",  "contains": "canon",   "ignore_case": true}     // substring, case-insensitive
{"field": "filename",      "regex":    "IMG_\\d{4}"}                       // RE2 syntax, case-sensitive
```

String comparisons (`eq`, `contains`, `starts_with`, `ends_with`, `regex`) are **case-sensitive by
default**; add a sibling `ignore_case: true` on the leaf to fold case (feature 15 — this replaced the
old `eq_ic` operator and the previously-always-on case-insensitivity of the substring operators).

**date:**

```jsonc
{"field": "captured_at", "year":        2024}
{"field": "captured_at", "month":       8}                   // 1–12
{"field": "captured_at", "season":      "summer"}            // spring|summer|autumn|winter
{"field": "captured_at", "date_range":  {"from": "2024-07-01", "to": "2024-08-31"}}
{"field": "captured_at", "time_range":  {"from": "06:00",      "to": "09:00"}}
```

`date_range` bounds are inclusive ISO-8601 date strings (no time component).  
`time_range` bounds are `HH:MM` 24-hour strings; a range crossing midnight (e.g. `{"from":
"22:00", "to": "03:00"}`) is valid and matches correctly.  
Season mapping: spring = Mar–May, summer = Jun–Aug, autumn = Sep–Nov, winter = Dec–Feb.

**bool:**

```jsonc
{"field": "is_owned", "eq": true}
```

**Presence check — any nullable field:**

```jsonc
{"field": "gps_lat",    "is_present": true}    // shorthand for "has GPS"
{"field": "captured_at","is_present": false}   // no capture date
```

---

## 3. Rule structure (unchanged)

A `RuleTaggingService` still owns N `rule_tagging_services` rows. Each row pairs **one predicate
tree** with **one `assign_tag`**. The rows are evaluated independently: multiple rows can match and
assign different tags in the same pipeline run. This is the right model — rows let users group
related tagging rules under one named service (e.g. "Camera rules") without forcing all of them to
produce the same tag.

```
RuleTaggingService "My rules"
  rule 1: (predicate tree A) → /Photos/Alps
  rule 2: (predicate tree B) → /Photos/2024/Summer
  rule 3: (predicate tree C) → /Camera/Fuji
```

---

## 4. Schema change

`rule_tagging_services.predicate` changes from `TEXT` to `JSONB`:

```sql
ALTER TABLE rule_tagging_services ALTER COLUMN predicate TYPE JSONB USING predicate::jsonb;
```

Existing text predicates (there are very few, and they follow a known format) are migrated at
schema update time by a one-off conversion in the migration script that converts the old
function-call strings to their JSON equivalents:

| Old text predicate            | Equivalent JSONB                                                  |
|-------------------------------|-------------------------------------------------------------------|
| `gps_within_bbox(a, b, c, d)` | `{"gps_bbox": {"lat_min":a,"lat_max":b,"lon_min":c,"lon_max":d}}` |
| `capture_year(Y)`             | `{"field":"captured_at","year":Y}`                                |
| `capture_month(M)`            | `{"field":"captured_at","month":M}`                               |
| `filename_contains("s")`      | `{"field":"filename","contains":"s"}`                             |

---

## 5. `PipelineInput` additions

The evaluator's input struct gains the fields needed to evaluate the new conditions. New fields
added to `PipelineInput` (all `Option` — absent when the picture lacks the data):

```rust
pub camera_brand:    Option<String>,
pub camera_model:    Option<String>,
pub focal_length_mm: Option<f64>,
pub f_number:        Option<f64>,
pub iso_speed:       Option<i32>,
pub exposure_time:   Option<f64>,   // num/den → f64 seconds; None if either column is NULL
pub orientation:     Option<i16>,
pub mime_type:       Option<String>,
pub file_size:       Option<i64>,
pub width:           Option<i32>,
pub height:          Option<i32>,
pub is_owned:        bool,           // derived: remote_picture_id IS NULL
```

---

## 6. Validation at creation / update time

Predicates are validated when a rule is created or updated (not at evaluation time):

- Structural validity: every node is a recognised form; no unknown keys.
- Type compatibility: the condition key matches the field's declared base type (e.g. `contains` on
  `iso_speed` is rejected).
- Range sanity: `min ≤ max` where both are provided; `month` is 1–12; `lat_min ≤ lat_max`;
  `lon_min ≤ lon_max`.
- Regex syntax: `regex` values must compile under RE2 rules.
- Depth limit: predicate trees are capped at 10 levels to prevent pathological inputs.

Validation errors are returned as structured messages identifying the invalid node (e.g.
`"rules[1].predicate.and[0]: field 'iso_speed' does not support condition 'contains'"`).

---

## 7. Evaluation (pure, no I/O)

`evaluate_rule` in `domain/pipeline.rs` is updated to accept the new `Predicate` type (replacing
the parsed text `Predicate` enum). Evaluation is a recursive match over the tree:

- `And`: short-circuits on first false child.
- `Or`: short-circuits on first true child.
- `Not`: inverts child result.
- `GpsBbox` / `GpsRadius`: use the existing bbox/radius logic on `gps_lat`/`gps_lng` from
  `PipelineInput`.
- `Field`: look up the named field on `PipelineInput`, apply the typed condition. A field that is
  `None` in the input evaluates `is_present: false` as `true` and all other conditions as `false`
  (i.e. a missing value never satisfies a value-based filter).

---

## 8. API changes

`POST /api/authenticated/tagging-services/rule` and `PATCH …/{id}` accept rules with a
`predicate` field that is now a JSON object instead of a string. The old string format is no
longer accepted. Validation (§6) runs server-side before persistence.

`GET` responses return the predicate as a JSON object.

---

## 9. What this does NOT change

- The `requires`/`excludes` gate on `TaggingService` (the service-level gate, not per-rule).
- `SegmentationTaggingService` — date-range segmentation remains a separate service type.
- The pipeline execution model (dirty-picture detection, wake model, per-source reconciliation).
- The `SharedTagMappingService`.
