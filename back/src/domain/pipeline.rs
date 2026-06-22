use crate::domain::tag::TagPath;
use crate::domain::tagging::{SegmentationRule, SharedTagMappingRule, TaggingService};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::Value;
use uuid::Uuid;

// ── Pipeline input ────────────────────────────────────────────────────────────

/// Input fed to the pipeline evaluator for a single picture.
#[derive(Debug, Clone)]
pub struct PipelineInput {
    pub picture_id: Uuid,
    pub captured_at: Option<NaiveDateTime>,
    pub ingested_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub filename: Option<String>,
    // ── Better-rules (feature 13) fields — all Option except the derived `is_owned`. ──
    pub camera_brand: Option<String>,
    pub camera_model: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub f_number: Option<f64>,
    pub iso_speed: Option<i32>,
    /// `exposure_time_num / exposure_time_den` in seconds; `None` if either column is NULL.
    pub exposure_time: Option<f64>,
    pub orientation: Option<i16>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Derived: `remote_picture_id IS NULL` (the picture is owned by this user).
    pub is_owned: bool,
    /// Tags currently assigned to this picture; updated in-memory as services run
    /// so later services in the pipeline see tags added by earlier ones.
    pub current_tags: Vec<TagPath>,
}

/// Tags to add as a result of evaluating one service against one picture.
#[derive(Debug, Clone, Default)]
pub struct ServiceResult {
    pub tags_to_add: Vec<TagPath>,
}

// ── should_run ────────────────────────────────────────────────────────────────

/// Return true if `service` should run for `input`.
///
/// Checks `enabled`, `requires` (all must match), and `excludes` (none must match).
/// Services are run for every dirty picture; label-based filtering is not needed
/// because the pipeline only runs pictures that need re-evaluation anyway.
pub fn should_run(service: &TaggingService, input: &PipelineInput) -> bool {
    if !service.enabled {
        return false;
    }
    // All required tags must be present (exact match or descendant).
    let satisfied = service.requires.iter().all(|req| {
        let req_path = TagPath::from_ltree(req);
        input
            .current_tags
            .iter()
            .any(|t| t == &req_path || t.ancestors().contains(&req_path))
    });
    // Any excluded tag suppresses this service.
    let excluded = service.excludes.iter().any(|exc| {
        let exc_path = TagPath::from_ltree(exc);
        input
            .current_tags
            .iter()
            .any(|t| t == &exc_path || t.ancestors().contains(&exc_path))
    });
    satisfied && !excluded
}

// ── Predicate (Rule service) ──────────────────────────────────────────────────
//
// A predicate is a recursive JSON tree (feature 13). It is stored as JSONB and parsed into the
// typed [`Predicate`] below at validation time (on create/update) and at evaluation time.
// See doc/features/13_better_rules.md.

/// Maximum nesting depth of a predicate tree (guards against pathological inputs).
const MAX_PREDICATE_DEPTH: usize = 10;

/// The base type of a queryable picture field, which gates the conditions it accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseType {
    Int,
    Float,
    Str,
    Date,
    Bool,
}

/// A queryable picture field. Each maps to a column / EXIF derivation and a [`BaseType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    CapturedAt,
    IngestedAt,
    UpdatedAt,
    GpsLat,
    GpsLng,
    GpsAlt,
    IsoSpeed,
    FNumber,
    FocalLengthMm,
    ExposureTime,
    Orientation,
    CameraBrand,
    CameraModel,
    Filename,
    MimeType,
    FileSize,
    Width,
    Height,
    IsOwned,
}

impl Field {
    fn parse(name: &str) -> Option<Field> {
        Some(match name {
            "captured_at" => Field::CapturedAt,
            "ingested_at" => Field::IngestedAt,
            "updated_at" => Field::UpdatedAt,
            "gps_lat" => Field::GpsLat,
            "gps_lng" => Field::GpsLng,
            "gps_alt" => Field::GpsAlt,
            "iso_speed" => Field::IsoSpeed,
            "f_number" => Field::FNumber,
            "focal_length_mm" => Field::FocalLengthMm,
            "exposure_time" => Field::ExposureTime,
            "orientation" => Field::Orientation,
            "camera_brand" => Field::CameraBrand,
            "camera_model" => Field::CameraModel,
            "filename" => Field::Filename,
            "mime_type" => Field::MimeType,
            "file_size" => Field::FileSize,
            "width" => Field::Width,
            "height" => Field::Height,
            "is_owned" => Field::IsOwned,
            _ => return None,
        })
    }

    fn base_type(self) -> BaseType {
        match self {
            Field::CapturedAt | Field::IngestedAt | Field::UpdatedAt => BaseType::Date,
            Field::GpsLat
            | Field::GpsLng
            | Field::FNumber
            | Field::FocalLengthMm
            | Field::ExposureTime => BaseType::Float,
            Field::GpsAlt
            | Field::IsoSpeed
            | Field::Orientation
            | Field::FileSize
            | Field::Width
            | Field::Height => BaseType::Int,
            Field::CameraBrand | Field::CameraModel | Field::Filename | Field::MimeType => {
                BaseType::Str
            }
            Field::IsOwned => BaseType::Bool,
        }
    }

    /// The picture's value for this field, as a comparable runtime value. `Bool` is always present.
    fn value<'a>(self, input: &'a PipelineInput) -> FieldValue<'a> {
        match self {
            Field::CapturedAt => FieldValue::Date(input.captured_at),
            Field::IngestedAt => FieldValue::Date(input.ingested_at),
            Field::UpdatedAt => FieldValue::Date(input.updated_at),
            Field::GpsLat => FieldValue::Float(input.gps_lat),
            Field::GpsLng => FieldValue::Float(input.gps_lng),
            Field::GpsAlt => FieldValue::Int(input.gps_alt.map(i64::from)),
            Field::IsoSpeed => FieldValue::Int(input.iso_speed.map(i64::from)),
            Field::FNumber => FieldValue::Float(input.f_number),
            Field::FocalLengthMm => FieldValue::Float(input.focal_length_mm),
            Field::ExposureTime => FieldValue::Float(input.exposure_time),
            Field::Orientation => FieldValue::Int(input.orientation.map(i64::from)),
            Field::CameraBrand => FieldValue::Str(input.camera_brand.as_deref()),
            Field::CameraModel => FieldValue::Str(input.camera_model.as_deref()),
            Field::Filename => FieldValue::Str(input.filename.as_deref()),
            Field::MimeType => FieldValue::Str(input.mime_type.as_deref()),
            Field::FileSize => FieldValue::Int(input.file_size),
            Field::Width => FieldValue::Int(input.width.map(i64::from)),
            Field::Height => FieldValue::Int(input.height.map(i64::from)),
            Field::IsOwned => FieldValue::Bool(input.is_owned),
        }
    }
}

/// A picture's runtime value for one [`Field`], used during evaluation.
enum FieldValue<'a> {
    Int(Option<i64>),
    Float(Option<f64>),
    Str(Option<&'a str>),
    Date(Option<NaiveDateTime>),
    Bool(bool),
}

impl FieldValue<'_> {
    /// Whether the underlying value is present. `Bool` is always present.
    fn is_present(&self) -> bool {
        match self {
            FieldValue::Int(v) => v.is_some(),
            FieldValue::Float(v) => v.is_some(),
            FieldValue::Str(v) => v.is_some(),
            FieldValue::Date(v) => v.is_some(),
            FieldValue::Bool(_) => true,
        }
    }

    /// Numeric value as f64 (Int/Float only).
    fn as_f64(&self) -> Option<f64> {
        match self {
            FieldValue::Int(v) => v.map(|n| n as f64),
            FieldValue::Float(v) => *v,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    fn parse(s: &str) -> Option<Season> {
        Some(match s {
            "spring" => Season::Spring,
            "summer" => Season::Summer,
            "autumn" => Season::Autumn,
            "winter" => Season::Winter,
            _ => return None,
        })
    }

    fn contains_month(self, month: u32) -> bool {
        match self {
            Season::Spring => (3..=5).contains(&month),
            Season::Summer => (6..=8).contains(&month),
            Season::Autumn => (9..=11).contains(&month),
            Season::Winter => month == 12 || month == 1 || month == 2,
        }
    }
}

/// A single typed condition applied to one field's value. A field predicate holds one or more of
/// these (e.g. `min` + `max`); all must match (AND).
#[derive(Debug, Clone)]
enum Condition {
    /// Presence check — valid on any field. `true` ⇒ value must be present.
    IsPresent(bool),
    NumEq(f64),
    NumMin(f64),
    NumMax(f64),
    StrEq(String),
    StrEqIc(String),
    StrContains(String),
    StrStartsWith(String),
    StrEndsWith(String),
    StrRegex(regex::Regex),
    Year(i32),
    Month(u32),
    Season(Season),
    DateRange {
        from: NaiveDateTime,
        to: NaiveDateTime,
    },
    TimeRange {
        from: NaiveTime,
        to: NaiveTime,
    },
    BoolEq(bool),
}

impl Condition {
    fn matches(&self, value: &FieldValue) -> bool {
        // Presence is the one condition that distinguishes absent from present.
        if let Condition::IsPresent(want) = self {
            return value.is_present() == *want;
        }
        // Any value-based condition on an absent value never matches.
        if !value.is_present() {
            return false;
        }
        match self {
            Condition::IsPresent(_) => unreachable!(),
            Condition::NumEq(n) => value.as_f64().map_or(false, |v| v == *n),
            Condition::NumMin(n) => value.as_f64().map_or(false, |v| v >= *n),
            Condition::NumMax(n) => value.as_f64().map_or(false, |v| v <= *n),
            Condition::StrEq(s) => matches!(value, FieldValue::Str(Some(v)) if v == s),
            Condition::StrEqIc(s) => {
                matches!(value, FieldValue::Str(Some(v)) if v.eq_ignore_ascii_case(s))
            }
            Condition::StrContains(s) => matches!(
                value,
                FieldValue::Str(Some(v)) if v.to_lowercase().contains(&s.to_lowercase())
            ),
            Condition::StrStartsWith(s) => matches!(
                value,
                FieldValue::Str(Some(v)) if v.to_lowercase().starts_with(&s.to_lowercase())
            ),
            Condition::StrEndsWith(s) => matches!(
                value,
                FieldValue::Str(Some(v)) if v.to_lowercase().ends_with(&s.to_lowercase())
            ),
            Condition::StrRegex(re) => matches!(value, FieldValue::Str(Some(v)) if re.is_match(v)),
            Condition::Year(y) => matches!(value, FieldValue::Date(Some(d)) if d.year() == *y),
            Condition::Month(m) => matches!(value, FieldValue::Date(Some(d)) if d.month() == *m),
            Condition::Season(s) => {
                matches!(value, FieldValue::Date(Some(d)) if s.contains_month(d.month()))
            }
            Condition::DateRange { from, to } => {
                matches!(value, FieldValue::Date(Some(d)) if d >= from && d <= to)
            }
            Condition::TimeRange { from, to } => match value {
                FieldValue::Date(Some(d)) => {
                    let t = d.time();
                    if from <= to {
                        t >= *from && t <= *to
                    } else {
                        // Range crosses midnight (e.g. 22:00 → 03:00).
                        t >= *from || t <= *to
                    }
                }
                _ => false,
            },
            Condition::BoolEq(b) => matches!(value, FieldValue::Bool(v) if v == b),
        }
    }
}

/// A structured rule predicate: a recursive tree of logical nodes, spatial predicates, and typed
/// field conditions.
#[derive(Debug, Clone)]
pub enum Predicate {
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
    GpsBbox {
        lat_min: f64,
        lat_max: f64,
        lon_min: f64,
        lon_max: f64,
    },
    GpsRadius {
        lat: f64,
        lng: f64,
        km: f64,
    },
    Field {
        field: Field,
        conds: Vec<Condition>,
    },
}

impl Predicate {
    /// Parse and validate a predicate tree from JSON. Errors carry a path to the offending node
    /// (e.g. `and[0]: field 'iso_speed' does not support condition 'contains'`).
    pub fn parse(value: &Value) -> Result<Self, String> {
        parse_node(value, "", 0)
    }

    /// Evaluate the predicate against the picture input (pure, no I/O).
    pub fn matches(&self, input: &PipelineInput) -> bool {
        match self {
            Predicate::And(children) => children.iter().all(|c| c.matches(input)),
            Predicate::Or(children) => children.iter().any(|c| c.matches(input)),
            Predicate::Not(child) => !child.matches(input),
            Predicate::GpsBbox {
                lat_min,
                lat_max,
                lon_min,
                lon_max,
            } => match (input.gps_lat, input.gps_lng) {
                (Some(lat), Some(lng)) => {
                    lat >= *lat_min && lat <= *lat_max && lng >= *lon_min && lng <= *lon_max
                }
                _ => false,
            },
            Predicate::GpsRadius { lat, lng, km } => match (input.gps_lat, input.gps_lng) {
                (Some(plat), Some(plng)) => haversine_km(*lat, *lng, plat, plng) <= *km,
                _ => false,
            },
            Predicate::Field { field, conds } => {
                let value = field.value(input);
                conds.iter().all(|c| c.matches(&value))
            }
        }
    }
}

/// Great-circle distance between two lat/lng points, in kilometres.
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6371.0; // mean Earth radius (km)
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

// ── Predicate parsing / validation ──────────────────────────────────────────────

fn parse_node(value: &Value, path: &str, depth: usize) -> Result<Predicate, String> {
    if depth > MAX_PREDICATE_DEPTH {
        return Err(format!(
            "{}: predicate nested too deeply (max {MAX_PREDICATE_DEPTH})",
            at(path)
        ));
    }
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{}: expected an object", at(path)))?;

    // Logical and spatial nodes are keyed by a single discriminator.
    if let Some(v) = obj.get("and") {
        reject_extra_keys(obj, &["and"], path)?;
        return Ok(Predicate::And(parse_children(v, path, "and", depth)?));
    }
    if let Some(v) = obj.get("or") {
        reject_extra_keys(obj, &["or"], path)?;
        return Ok(Predicate::Or(parse_children(v, path, "or", depth)?));
    }
    if let Some(v) = obj.get("not") {
        reject_extra_keys(obj, &["not"], path)?;
        let child = parse_node(v, &child_path(path, "not"), depth + 1)?;
        return Ok(Predicate::Not(Box::new(child)));
    }
    if let Some(v) = obj.get("gps_bbox") {
        reject_extra_keys(obj, &["gps_bbox"], path)?;
        return parse_gps_bbox(v, &child_path(path, "gps_bbox"));
    }
    if let Some(v) = obj.get("gps_radius") {
        reject_extra_keys(obj, &["gps_radius"], path)?;
        return parse_gps_radius(v, &child_path(path, "gps_radius"));
    }
    if let Some(field_val) = obj.get("field") {
        return parse_field(obj, field_val, path);
    }

    Err(format!(
        "{}: unrecognised predicate node (expected one of: and, or, not, gps_bbox, gps_radius, field)",
        at(path)
    ))
}

fn parse_children(
    value: &Value,
    path: &str,
    key: &str,
    depth: usize,
) -> Result<Vec<Predicate>, String> {
    let arr = value
        .as_array()
        .ok_or_else(|| format!("{}: '{key}' must be an array", at(path)))?;
    arr.iter()
        .enumerate()
        .map(|(i, child)| parse_node(child, &format!("{}{key}[{i}]", prefix(path)), depth + 1))
        .collect()
}

fn parse_gps_bbox(value: &Value, path: &str) -> Result<Predicate, String> {
    let lat_min = num_field(value, "lat_min", path)?;
    let lat_max = num_field(value, "lat_max", path)?;
    let lon_min = num_field(value, "lon_min", path)?;
    let lon_max = num_field(value, "lon_max", path)?;
    if lat_min > lat_max {
        return Err(format!("{}: lat_min must be ≤ lat_max", at(path)));
    }
    if lon_min > lon_max {
        return Err(format!("{}: lon_min must be ≤ lon_max", at(path)));
    }
    Ok(Predicate::GpsBbox {
        lat_min,
        lat_max,
        lon_min,
        lon_max,
    })
}

fn parse_gps_radius(value: &Value, path: &str) -> Result<Predicate, String> {
    let lat = num_field(value, "lat", path)?;
    let lng = num_field(value, "lng", path)?;
    let km = num_field(value, "km", path)?;
    if km < 0.0 {
        return Err(format!("{}: km must be ≥ 0", at(path)));
    }
    Ok(Predicate::GpsRadius { lat, lng, km })
}

fn parse_field(
    obj: &serde_json::Map<String, Value>,
    field_val: &Value,
    path: &str,
) -> Result<Predicate, String> {
    let name = field_val
        .as_str()
        .ok_or_else(|| format!("{}: 'field' must be a string", at(path)))?;
    let field =
        Field::parse(name).ok_or_else(|| format!("{}: unknown field '{name}'", at(path)))?;
    let bt = field.base_type();

    let mut conds: Vec<Condition> = Vec::new();
    for (key, v) in obj {
        if key == "field" {
            continue;
        }
        let cond = parse_condition(field, bt, key, v, name, path)?;
        conds.push(cond);
    }
    if conds.is_empty() {
        return Err(format!("{}: field '{name}' has no condition", at(path)));
    }
    Ok(Predicate::Field { field, conds })
}

fn parse_condition(
    _field: Field,
    bt: BaseType,
    key: &str,
    v: &Value,
    field_name: &str,
    path: &str,
) -> Result<Condition, String> {
    let unsupported = || {
        format!(
            "{}: field '{field_name}' does not support condition '{key}'",
            at(path)
        )
    };
    match key {
        "is_present" => {
            let b = v
                .as_bool()
                .ok_or_else(|| format!("{}: 'is_present' must be a boolean", at(path)))?;
            Ok(Condition::IsPresent(b))
        }
        "eq" if bt == BaseType::Bool => {
            let b = v
                .as_bool()
                .ok_or_else(|| format!("{}: 'eq' must be a boolean", at(path)))?;
            Ok(Condition::BoolEq(b))
        }
        "eq" if matches!(bt, BaseType::Int | BaseType::Float) => {
            Ok(Condition::NumEq(num_value(v, key, path)?))
        }
        "min" if matches!(bt, BaseType::Int | BaseType::Float) => {
            Ok(Condition::NumMin(num_value(v, key, path)?))
        }
        "max" if matches!(bt, BaseType::Int | BaseType::Float) => {
            Ok(Condition::NumMax(num_value(v, key, path)?))
        }
        "eq" if bt == BaseType::Str => Ok(Condition::StrEq(str_value(v, key, path)?)),
        "eq_ic" if bt == BaseType::Str => Ok(Condition::StrEqIc(str_value(v, key, path)?)),
        "contains" if bt == BaseType::Str => Ok(Condition::StrContains(str_value(v, key, path)?)),
        "starts_with" if bt == BaseType::Str => {
            Ok(Condition::StrStartsWith(str_value(v, key, path)?))
        }
        "ends_with" if bt == BaseType::Str => Ok(Condition::StrEndsWith(str_value(v, key, path)?)),
        "regex" if bt == BaseType::Str => {
            let pat = str_value(v, key, path)?;
            let re =
                regex::Regex::new(&pat).map_err(|e| format!("{}: invalid regex: {e}", at(path)))?;
            Ok(Condition::StrRegex(re))
        }
        "year" if bt == BaseType::Date => Ok(Condition::Year(int_value(v, key, path)? as i32)),
        "month" if bt == BaseType::Date => {
            let m = int_value(v, key, path)?;
            if !(1..=12).contains(&m) {
                return Err(format!("{}: month must be 1–12", at(path)));
            }
            Ok(Condition::Month(m as u32))
        }
        "season" if bt == BaseType::Date => {
            let s = str_value(v, key, path)?;
            let season = Season::parse(&s).ok_or_else(|| {
                format!("{}: season must be spring|summer|autumn|winter", at(path))
            })?;
            Ok(Condition::Season(season))
        }
        "date_range" if bt == BaseType::Date => {
            // Bounds may be date-only (`YYYY-MM-DD`, treated as full-day) or a NaiveDateTime
            // (`YYYY-MM-DDTHH:MM:SS`) when the user sets a time.
            let (from_s, to_s) = from_to(v, path)?;
            let from = parse_date_bound(&from_s, false).ok_or_else(|| {
                format!(
                    "{}: date_range.from must be YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS",
                    at(path)
                )
            })?;
            let to = parse_date_bound(&to_s, true).ok_or_else(|| {
                format!(
                    "{}: date_range.to must be YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS",
                    at(path)
                )
            })?;
            if from > to {
                return Err(format!("{}: date_range.from must be ≤ to", at(path)));
            }
            Ok(Condition::DateRange { from, to })
        }
        "time_range" if bt == BaseType::Date => {
            let (from_s, to_s) = from_to(v, path)?;
            let from = NaiveTime::parse_from_str(&from_s, "%H:%M")
                .map_err(|_| format!("{}: time_range.from must be HH:MM", at(path)))?;
            let to = NaiveTime::parse_from_str(&to_s, "%H:%M")
                .map_err(|_| format!("{}: time_range.to must be HH:MM", at(path)))?;
            Ok(Condition::TimeRange { from, to })
        }
        _ => Err(unsupported()),
    }
}

// ── Parsing helpers ─────────────────────────────────────────────────────────────

/// Parse a `date_range` bound: a full `NaiveDateTime`, or a date-only string expanded to the start
/// (`is_end = false` → 00:00:00) or end (`is_end = true` → 23:59:59) of that day.
fn parse_date_bound(s: &str, is_end: bool) -> Option<NaiveDateTime> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt);
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    if is_end {
        d.and_hms_opt(23, 59, 59)
    } else {
        d.and_hms_opt(0, 0, 0)
    }
}

fn from_to(v: &Value, path: &str) -> Result<(String, String), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{}: expected an object with 'from' and 'to'", at(path)))?;
    let from = obj
        .get("from")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("{}: missing string 'from'", at(path)))?;
    let to = obj
        .get("to")
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("{}: missing string 'to'", at(path)))?;
    Ok((from.to_string(), to.to_string()))
}

fn num_field(value: &Value, key: &str, path: &str) -> Result<f64, String> {
    value
        .get(key)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("{}: missing or non-numeric '{key}'", at(path)))
}

fn num_value(v: &Value, key: &str, path: &str) -> Result<f64, String> {
    v.as_f64()
        .ok_or_else(|| format!("{}: '{key}' must be a number", at(path)))
}

fn int_value(v: &Value, key: &str, path: &str) -> Result<i64, String> {
    v.as_i64()
        .ok_or_else(|| format!("{}: '{key}' must be an integer", at(path)))
}

fn str_value(v: &Value, key: &str, path: &str) -> Result<String, String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{}: '{key}' must be a string", at(path)))
}

fn reject_extra_keys(
    obj: &serde_json::Map<String, Value>,
    allowed: &[&str],
    path: &str,
) -> Result<(), String> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "{}: unexpected key '{key}' (a '{}' node takes no other keys)",
                at(path),
                allowed[0]
            ));
        }
    }
    Ok(())
}

/// Render a node path for an error message; the root has the implicit name "predicate".
fn at(path: &str) -> String {
    if path.is_empty() {
        "predicate".to_string()
    } else {
        path.to_string()
    }
}

fn prefix(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        format!("{path}.")
    }
}

fn child_path(path: &str, key: &str) -> String {
    format!("{}{key}", prefix(path))
}

// ── Evaluators ────────────────────────────────────────────────────────────────

/// Evaluate a segmentation service against an input — pure, no I/O.
pub fn evaluate_segmentation(rules: &[SegmentationRule], input: &PipelineInput) -> ServiceResult {
    let mut tags_to_add = Vec::new();
    if let Some(captured_at) = input.captured_at {
        for rule in rules {
            if captured_at >= rule.date_start && captured_at <= rule.date_end {
                tags_to_add.push(TagPath::from_ltree(&rule.assign_tag));
            }
        }
    }
    ServiceResult { tags_to_add }
}

/// Evaluate a shared-tag-mapping service — pure, no I/O.
pub fn evaluate_shared_tag_mapping(
    rules: &[SharedTagMappingRule],
    incoming_share_ids: &[Uuid],
) -> ServiceResult {
    let tags_to_add = rules
        .iter()
        .filter(|r| !r.is_broken && incoming_share_ids.contains(&r.incoming_share_id))
        .map(|r| TagPath::from_ltree(&r.assign_tag))
        .collect();
    ServiceResult { tags_to_add }
}

/// Evaluate a rule service against an input — pure, no I/O.
///
/// Rules whose predicate fails to parse are silently skipped (the predicate is
/// validated at creation time, so this should not happen in practice).
pub fn evaluate_rule(
    rules: &[crate::domain::tagging::RuleTaggingRule],
    input: &PipelineInput,
) -> ServiceResult {
    let mut tags_to_add = Vec::new();
    for rule in rules {
        match Predicate::parse(&rule.predicate) {
            Ok(pred) if pred.matches(input) => {
                tags_to_add.push(TagPath::from_ltree(&rule.assign_tag));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    predicate = %rule.predicate,
                    error = %e,
                    "rule predicate failed to parse during evaluation — skipping"
                );
            }
        }
    }
    ServiceResult { tags_to_add }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tagging::{
        RuleTaggingRule, SegmentationRule, ServiceType, SharedTagMappingRule, TaggingService,
    };
    use serde_json::json;
    use uuid::Uuid;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn service(stype: ServiceType, requires: &[&str], excludes: &[&str]) -> TaggingService {
        TaggingService {
            id: Uuid::new_v4(),
            owner_id: Uuid::new_v4(),
            name: String::new(),
            service_type: stype,
            requires: requires.iter().map(|s| s.to_string()).collect(),
            excludes: excludes.iter().map(|s| s.to_string()).collect(),
            enabled: true,
            position: 0,
            last_invalidated_at: dt("2024-01-01 00:00:00"),
            last_error_at: None,
            last_error_msg: None,
            created_at: dt("2024-01-01 00:00:00"),
            updated_at: dt("2024-01-01 00:00:00"),
        }
    }

    /// A blank input with every field absent and `is_owned = true`. Tests mutate the fields
    /// they care about.
    fn blank_input() -> PipelineInput {
        PipelineInput {
            picture_id: Uuid::new_v4(),
            captured_at: None,
            ingested_at: None,
            updated_at: None,
            gps_lat: None,
            gps_lng: None,
            gps_alt: None,
            filename: None,
            camera_brand: None,
            camera_model: None,
            focal_length_mm: None,
            f_number: None,
            iso_speed: None,
            exposure_time: None,
            orientation: None,
            mime_type: None,
            file_size: None,
            width: None,
            height: None,
            is_owned: true,
            current_tags: vec![],
        }
    }

    fn input(tags: &[&str], captured_at: Option<&str>) -> PipelineInput {
        PipelineInput {
            captured_at: captured_at.map(dt),
            current_tags: tags.iter().map(|s| TagPath::from_ltree(*s)).collect(),
            ..blank_input()
        }
    }

    fn input_with_gps(lat: f64, lng: f64) -> PipelineInput {
        PipelineInput {
            gps_lat: Some(lat),
            gps_lng: Some(lng),
            ..blank_input()
        }
    }

    // ── should_run ────────────────────────────────────────────────────────────

    #[test]
    fn disabled_service_never_runs() {
        let mut svc = service(ServiceType::Segmentation, &[], &[]);
        svc.enabled = false;
        assert!(!should_run(&svc, &input(&["Photos"], None)));
    }

    #[test]
    fn requires_exact_tag_match() {
        let svc = service(ServiceType::Rule, &["Photos"], &[]);
        assert!(!should_run(&svc, &input(&["Images"], None)));
        assert!(should_run(&svc, &input(&["Photos"], None)));
    }

    #[test]
    fn requires_satisfied_by_ancestor() {
        let svc = service(ServiceType::Rule, &["Photos"], &[]);
        assert!(should_run(&svc, &input(&["Photos.Travel.Alps"], None)));
    }

    #[test]
    fn excludes_suppresses_service() {
        let svc = service(ServiceType::Rule, &[], &["Images"]);
        assert!(should_run(&svc, &input(&["Photos"], None)));
        assert!(!should_run(&svc, &input(&["Images"], None)));
    }

    #[test]
    fn excludes_triggered_by_ancestor() {
        let svc = service(ServiceType::Rule, &[], &["Images"]);
        assert!(!should_run(&svc, &input(&["Images.Icons"], None)));
    }

    #[test]
    fn all_requires_must_be_present() {
        let svc = service(ServiceType::Rule, &["Photos", "Travel"], &[]);
        assert!(!should_run(&svc, &input(&["Photos"], None)));
        assert!(should_run(&svc, &input(&["Photos", "Travel"], None)));
    }

    // ── Predicate::parse — structural validity ──────────────────────────────────

    #[test]
    fn parse_empty_and_or() {
        assert!(Predicate::parse(&json!({"and": []})).is_ok());
        assert!(Predicate::parse(&json!({"or": []})).is_ok());
    }

    #[test]
    fn empty_and_always_matches_empty_or_never_matches() {
        let always = Predicate::parse(&json!({"and": []})).unwrap();
        let never = Predicate::parse(&json!({"or": []})).unwrap();
        assert!(always.matches(&blank_input()));
        assert!(!never.matches(&blank_input()));
    }

    #[test]
    fn parse_rejects_unknown_node() {
        assert!(Predicate::parse(&json!({"frobnicate": 3})).is_err());
    }

    #[test]
    fn parse_rejects_non_object() {
        assert!(Predicate::parse(&json!(42)).is_err());
        assert!(Predicate::parse(&json!("hi")).is_err());
    }

    #[test]
    fn parse_rejects_extra_keys_on_logical() {
        assert!(Predicate::parse(&json!({"and": [], "or": []})).is_err());
        assert!(Predicate::parse(&json!({"not": {"and": []}, "x": 1})).is_err());
    }

    #[test]
    fn parse_rejects_unknown_field() {
        assert!(Predicate::parse(&json!({"field": "nope", "eq": 1})).is_err());
    }

    #[test]
    fn parse_rejects_field_without_condition() {
        assert!(Predicate::parse(&json!({"field": "iso_speed"})).is_err());
    }

    #[test]
    fn parse_rejects_type_incompatible_condition() {
        // `contains` on a numeric field is rejected.
        assert!(Predicate::parse(&json!({"field": "iso_speed", "contains": "x"})).is_err());
        // `min` on a string field is rejected.
        assert!(Predicate::parse(&json!({"field": "camera_brand", "min": 3})).is_err());
        // `year` on a non-date field is rejected.
        assert!(Predicate::parse(&json!({"field": "width", "year": 2024})).is_err());
    }

    #[test]
    fn parse_rejects_bad_ranges() {
        assert!(Predicate::parse(&json!({"field": "captured_at", "month": 13})).is_err());
        assert!(
            Predicate::parse(&json!({"gps_bbox": {"lat_min": 46.0, "lat_max": 45.0, "lon_min": 4.0, "lon_max": 5.0}}))
                .is_err()
        );
        assert!(
            Predicate::parse(&json!({"field": "captured_at", "date_range": {"from": "2024-08-31", "to": "2024-07-01"}}))
                .is_err()
        );
    }

    #[test]
    fn parse_rejects_invalid_regex() {
        assert!(Predicate::parse(&json!({"field": "filename", "regex": "IMG_("})).is_err());
    }

    #[test]
    fn parse_rejects_too_deep() {
        // Build 12 levels of nested `not`.
        let mut v = json!({"and": []});
        for _ in 0..12 {
            v = json!({"not": v});
        }
        assert!(Predicate::parse(&v).is_err());
    }

    #[test]
    fn parse_error_carries_node_path() {
        let err = Predicate::parse(&json!({"and": [{"field": "iso_speed", "contains": "x"}]}))
            .unwrap_err();
        assert!(err.contains("and[0]"), "error was: {err}");
        assert!(err.contains("contains"), "error was: {err}");
    }

    // ── Predicate::matches — logical composition ────────────────────────────────

    #[test]
    fn and_or_not_compose() {
        let mut inp = blank_input();
        inp.iso_speed = Some(400);
        inp.camera_brand = Some("Fujifilm".to_string());

        let pred = Predicate::parse(&json!({
            "and": [
                {"field": "iso_speed", "min": 100, "max": 800},
                {"or": [
                    {"field": "camera_brand", "eq_ic": "fujifilm"},
                    {"field": "camera_brand", "eq": "Canon"}
                ]},
                {"not": {"field": "is_owned", "eq": false}}
            ]
        }))
        .unwrap();
        assert!(pred.matches(&inp));

        inp.iso_speed = Some(1600); // outside [100, 800]
        assert!(!pred.matches(&inp));
    }

    // ── numeric conditions ──────────────────────────────────────────────────────

    #[test]
    fn numeric_eq_min_max() {
        let mut inp = blank_input();
        inp.iso_speed = Some(400);
        assert!(
            Predicate::parse(&json!({"field": "iso_speed", "eq": 400}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "iso_speed", "eq": 200}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            Predicate::parse(&json!({"field": "iso_speed", "min": 100, "max": 800}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "iso_speed", "max": 200}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn float_conditions() {
        let mut inp = blank_input();
        inp.f_number = Some(1.8);
        assert!(
            Predicate::parse(&json!({"field": "f_number", "max": 2.8}))
                .unwrap()
                .matches(&inp)
        );
        inp.f_number = Some(4.0);
        assert!(
            !Predicate::parse(&json!({"field": "f_number", "max": 2.8}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn exposure_time_field() {
        let mut inp = blank_input();
        inp.exposure_time = Some(0.5);
        assert!(
            Predicate::parse(&json!({"field": "exposure_time", "min": 0.5}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn missing_numeric_never_matches_value_condition() {
        let inp = blank_input(); // iso_speed = None
        assert!(
            !Predicate::parse(&json!({"field": "iso_speed", "min": 100}))
                .unwrap()
                .matches(&inp)
        );
    }

    // ── string conditions ───────────────────────────────────────────────────────

    #[test]
    fn string_conditions() {
        let mut inp = blank_input();
        inp.mime_type = Some("image/heic".to_string());
        assert!(
            Predicate::parse(&json!({"field": "mime_type", "eq": "image/heic"}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "mime_type", "eq": "image/HEIC"}))
                .unwrap()
                .matches(&inp)
        );

        inp.camera_brand = Some("FUJIFILM".to_string());
        assert!(
            Predicate::parse(&json!({"field": "camera_brand", "eq_ic": "fujifilm"}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            Predicate::parse(&json!({"field": "camera_brand", "contains": "fuji"}))
                .unwrap()
                .matches(&inp)
        );

        inp.filename = Some("IMG_0421.jpg".to_string());
        assert!(
            Predicate::parse(&json!({"field": "filename", "regex": "IMG_\\d{4}"}))
                .unwrap()
                .matches(&inp)
        );
        inp.filename = Some("photo.jpg".to_string());
        assert!(
            !Predicate::parse(&json!({"field": "filename", "regex": "IMG_\\d{4}"}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn starts_with_ends_with() {
        let mut inp = blank_input();
        inp.filename = Some("IMG_0421.HEIC".to_string());
        assert!(
            Predicate::parse(&json!({"field": "filename", "starts_with": "img_"}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "filename", "starts_with": "DSC"}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            Predicate::parse(&json!({"field": "filename", "ends_with": ".heic"}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "filename", "ends_with": ".jpg"}))
                .unwrap()
                .matches(&inp)
        );
        // Not valid on a numeric field.
        assert!(Predicate::parse(&json!({"field": "iso_speed", "starts_with": "4"})).is_err());
    }

    // ── date conditions ─────────────────────────────────────────────────────────

    #[test]
    fn ingested_and_updated_date_fields() {
        let mut inp = blank_input();
        inp.ingested_at = Some(dt("2024-03-10 09:00:00"));
        inp.updated_at = Some(dt("2025-01-02 18:00:00"));
        assert!(
            Predicate::parse(&json!({"field": "ingested_at", "year": 2024}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            Predicate::parse(&json!({"field": "updated_at", "year": 2025}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "updated_at", "year": 2024}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn date_range_with_time_bounds() {
        let inp = input(&[], Some("2024-08-15 08:30:00"));
        // Date-only bounds expand to full days (inclusive).
        let day = Predicate::parse(&json!({"field": "captured_at", "date_range": {"from": "2024-08-15", "to": "2024-08-15"}})).unwrap();
        assert!(day.matches(&inp));
        // Datetime bounds narrow within the day.
        let narrow = Predicate::parse(&json!({"field": "captured_at", "date_range": {"from": "2024-08-15T09:00:00", "to": "2024-08-15T17:00:00"}})).unwrap();
        assert!(!narrow.matches(&inp)); // 08:30 is before 09:00
        let later = input(&[], Some("2024-08-15 12:00:00"));
        assert!(narrow.matches(&later));
    }

    #[test]
    fn date_year_month_season() {
        let inp = input(&[], Some("2024-08-05 10:00:00"));
        assert!(
            Predicate::parse(&json!({"field": "captured_at", "year": 2024}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "captured_at", "year": 2023}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            Predicate::parse(&json!({"field": "captured_at", "month": 8}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            Predicate::parse(&json!({"field": "captured_at", "season": "summer"}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "captured_at", "season": "winter"}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn date_range_inclusive() {
        let inp = input(&[], Some("2024-08-15 12:00:00"));
        let pred = Predicate::parse(&json!({"field": "captured_at", "date_range": {"from": "2024-07-01", "to": "2024-08-31"}})).unwrap();
        assert!(pred.matches(&inp));
        let outside = input(&[], Some("2024-09-01 12:00:00"));
        assert!(!pred.matches(&outside));
    }

    #[test]
    fn time_range_normal_and_wrapping() {
        let morning = input(&[], Some("2024-08-15 07:30:00"));
        let pred = Predicate::parse(
            &json!({"field": "captured_at", "time_range": {"from": "06:00", "to": "09:00"}}),
        )
        .unwrap();
        assert!(pred.matches(&morning));

        // Range crossing midnight.
        let night = input(&[], Some("2024-08-15 23:30:00"));
        let wrap = Predicate::parse(
            &json!({"field": "captured_at", "time_range": {"from": "22:00", "to": "03:00"}}),
        )
        .unwrap();
        assert!(wrap.matches(&night));
        let day = input(&[], Some("2024-08-15 12:00:00"));
        assert!(!wrap.matches(&day));
    }

    // ── bool & presence ─────────────────────────────────────────────────────────

    #[test]
    fn bool_field() {
        let mut inp = blank_input();
        inp.is_owned = true;
        assert!(
            Predicate::parse(&json!({"field": "is_owned", "eq": true}))
                .unwrap()
                .matches(&inp)
        );
        inp.is_owned = false;
        assert!(
            !Predicate::parse(&json!({"field": "is_owned", "eq": true}))
                .unwrap()
                .matches(&inp)
        );
    }

    #[test]
    fn presence_check() {
        let mut inp = blank_input();
        assert!(
            Predicate::parse(&json!({"field": "gps_lat", "is_present": false}))
                .unwrap()
                .matches(&inp)
        );
        assert!(
            !Predicate::parse(&json!({"field": "gps_lat", "is_present": true}))
                .unwrap()
                .matches(&inp)
        );
        inp.gps_lat = Some(45.0);
        assert!(
            Predicate::parse(&json!({"field": "gps_lat", "is_present": true}))
                .unwrap()
                .matches(&inp)
        );
    }

    // ── spatial predicates ──────────────────────────────────────────────────────

    #[test]
    fn gps_bbox_matches() {
        let pred = Predicate::parse(&json!({"gps_bbox": {"lat_min": 45.0, "lat_max": 46.0, "lon_min": 4.0, "lon_max": 5.0}})).unwrap();
        assert!(pred.matches(&input_with_gps(45.5, 4.5)));
        assert!(!pred.matches(&input_with_gps(47.0, 4.5)));
        assert!(!pred.matches(&blank_input()));
    }

    #[test]
    fn gps_radius_matches() {
        // Paris centre; a point ~1 km away is within 50 km, a point ~500 km away is not.
        let pred =
            Predicate::parse(&json!({"gps_radius": {"lat": 48.8566, "lng": 2.3522, "km": 50.0}}))
                .unwrap();
        assert!(pred.matches(&input_with_gps(48.86, 2.35)));
        assert!(!pred.matches(&input_with_gps(43.6, 1.44))); // Toulouse, ~590 km
    }

    // ── evaluate_segmentation ─────────────────────────────────────────────────

    fn seg_rule(assign: &str, start: &str, end: &str) -> SegmentationRule {
        SegmentationRule {
            id: Uuid::new_v4(),
            service_id: Uuid::new_v4(),
            name: assign.to_string(),
            date_start: dt(start),
            date_end: dt(end),
            assign_tag: assign.to_string(),
            parent_segment_id: None,
        }
    }

    #[test]
    fn segmentation_assigns_tag_in_range() {
        let rules = vec![seg_rule(
            "Photos.Trip",
            "2024-08-01 00:00:00",
            "2024-08-14 23:59:59",
        )];
        let result = evaluate_segmentation(&rules, &input(&[], Some("2024-08-05 10:00:00")));
        assert_eq!(result.tags_to_add, vec![TagPath::from_ltree("Photos.Trip")]);
    }

    #[test]
    fn segmentation_no_tag_outside_range() {
        let rules = vec![seg_rule(
            "Photos.Trip",
            "2024-08-01 00:00:00",
            "2024-08-14 23:59:59",
        )];
        let result = evaluate_segmentation(&rules, &input(&[], Some("2024-09-01 00:00:00")));
        assert!(result.tags_to_add.is_empty());
    }

    #[test]
    fn segmentation_no_captured_at_produces_no_tags() {
        let rules = vec![seg_rule(
            "Photos.Trip",
            "2024-08-01 00:00:00",
            "2024-08-14 23:59:59",
        )];
        let result = evaluate_segmentation(&rules, &input(&[], None));
        assert!(result.tags_to_add.is_empty());
    }

    // ── evaluate_shared_tag_mapping ───────────────────────────────────────────

    fn mapping_rule(incoming_share_id: Uuid, assign: &str, broken: bool) -> SharedTagMappingRule {
        SharedTagMappingRule {
            id: Uuid::new_v4(),
            service_id: Uuid::new_v4(),
            incoming_share_id,
            assign_tag: assign.to_string(),
            is_broken: broken,
        }
    }

    #[test]
    fn mapping_assigns_tag_for_matching_share() {
        let share_id = Uuid::new_v4();
        let rules = vec![mapping_rule(share_id, "Photos.Holidays.2024", false)];
        let result = evaluate_shared_tag_mapping(&rules, &[share_id]);
        assert_eq!(
            result.tags_to_add,
            vec![TagPath::from_ltree("Photos.Holidays.2024")]
        );
    }

    #[test]
    fn mapping_skips_broken_rules() {
        let share_id = Uuid::new_v4();
        let rules = vec![mapping_rule(share_id, "Photos.Holidays.2024", true)];
        let result = evaluate_shared_tag_mapping(&rules, &[share_id]);
        assert!(result.tags_to_add.is_empty());
    }

    // ── evaluate_rule ─────────────────────────────────────────────────────────

    fn rule(predicate: Value, assign: &str) -> RuleTaggingRule {
        RuleTaggingRule {
            id: Uuid::new_v4(),
            service_id: Uuid::new_v4(),
            predicate,
            assign_tag: assign.to_string(),
            position: 0,
        }
    }

    #[test]
    fn evaluate_rule_gps_match() {
        let rules = vec![rule(
            json!({"gps_bbox": {"lat_min": 45.0, "lat_max": 46.0, "lon_min": 4.0, "lon_max": 5.0}}),
            "Photos.Alps",
        )];
        let result = evaluate_rule(&rules, &input_with_gps(45.5, 4.5));
        assert_eq!(result.tags_to_add, vec![TagPath::from_ltree("Photos.Alps")]);
    }

    #[test]
    fn evaluate_rule_no_match() {
        let rules = vec![rule(
            json!({"field": "captured_at", "year": 2023}),
            "Photos.Y2023",
        )];
        let result = evaluate_rule(&rules, &input(&[], Some("2024-06-01 12:00:00")));
        assert!(result.tags_to_add.is_empty());
    }

    #[test]
    fn evaluate_rule_skips_invalid_predicate() {
        let rules = vec![rule(json!({"bogus": true}), "Photos.Bad")];
        let result = evaluate_rule(&rules, &blank_input());
        assert!(result.tags_to_add.is_empty());
    }
}
