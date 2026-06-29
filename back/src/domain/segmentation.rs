//! Calendar segmentation (feature 20): a `captured_at`→tag **partition operator**.
//!
//! A [`SegmentationConfig`] is an ordered, flat list of [`Band`]s. For a picture, the first
//! covering band wins and its template renders a single tag path under `root_tag`. Everything here
//! is a pure function of `captured_at` + the config — see `doc/features/20_calendar_segmentation.md`.

use crate::domain::tag::TagPath;
use chrono::{Datelike, Days, Months, NaiveDate, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};

// ── Config schema (§3) ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentationConfig {
    pub version: u32,
    /// ltree wire-form root; every band's path hangs under this.
    pub root_tag: String,
    #[serde(default)]
    pub hemisphere: Hemisphere,
    #[serde(default)]
    pub catch_all: Option<CatchAll>,
    #[serde(default)]
    pub bands: Vec<Band>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hemisphere {
    #[default]
    North,
    South,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatchAll {
    /// Single ltree label appended to `root_tag`.
    pub name: String,
    /// `true` ⇒ pictures with `captured_at = NULL` land here too.
    pub include_undated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Band {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub template: String,
    #[serde(default)]
    pub parts: std::collections::HashMap<String, PartConfig>,
    #[serde(default)]
    pub offset: Option<Offset>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartConfig {
    #[serde(default)]
    pub stride: Option<u32>,
    #[serde(default)]
    pub format: Option<PartFormat>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PartFormat {
    #[serde(default)]
    pub numeric: Option<bool>,
    #[serde(default)]
    pub pad: Option<u32>,
    #[serde(default)]
    pub abbrev: Option<bool>,
    #[serde(default)]
    pub case: Option<Case>,
    #[serde(default)]
    pub bound: Option<Bound>,
    #[serde(default)]
    pub range_sep: Option<String>,
    #[serde(default)]
    pub inclusive_end: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Case {
    Lower,
    Upper,
    Pascal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bound {
    Start,
    End,
    Range,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Offset {
    #[serde(default)]
    pub months: Option<i64>,
    #[serde(default)]
    pub days: Option<i64>,
    #[serde(default)]
    pub hours: Option<i64>,
    #[serde(default)]
    pub minutes: Option<i64>,
}

// ── Placeholder catalog (§4.1) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    Year,
    IsoYear,
    Quarter,
    Season,
    Month,
    Week,
    Day,
    Weekday,
    Daypart,
}

impl Placeholder {
    fn parse(name: &str) -> Option<Placeholder> {
        Some(match name {
            "year" => Placeholder::Year,
            "iso_year" => Placeholder::IsoYear,
            "quarter" => Placeholder::Quarter,
            "season" => Placeholder::Season,
            "month" => Placeholder::Month,
            "week" => Placeholder::Week,
            "day" => Placeholder::Day,
            "weekday" => Placeholder::Weekday,
            "daypart" => Placeholder::Daypart,
            _ => return None,
        })
    }

    /// Whether the field has named forms (so `numeric: false` / `abbrev` are valid).
    fn has_named_form(self) -> bool {
        matches!(
            self,
            Placeholder::Season | Placeholder::Month | Placeholder::Weekday | Placeholder::Daypart
        )
    }

    /// Default rendering: only `season`/`weekday`/`daypart` default to their name (§4.1 table);
    /// `month` has a named form but renders numerically by default.
    fn numeric_by_default(self) -> bool {
        !matches!(
            self,
            Placeholder::Season | Placeholder::Weekday | Placeholder::Daypart
        )
    }
}

// ── Template parsing (§4) ─────────────────────────────────────────────────────────

/// One dot-separated level of a template: a sequence of literal text and placeholder names.
#[derive(Debug, Clone)]
struct Level {
    parts: Vec<TemplatePart>,
}

#[derive(Debug, Clone)]
enum TemplatePart {
    Literal(String),
    Placeholder(Placeholder),
}

/// Parse a template into dot-separated levels, returning an error string (with no band index — the
/// caller adds that) on a malformed level / unknown placeholder.
fn parse_template(template: &str) -> Result<Vec<Level>, String> {
    let mut levels = Vec::new();
    for raw_level in template.split('.') {
        let parts = parse_level(raw_level)?;
        // Each level must carry at least one placeholder or alphanumeric literal char.
        let has_content = parts.iter().any(|p| match p {
            TemplatePart::Placeholder(_) => true,
            TemplatePart::Literal(s) => s.chars().any(|c| c.is_ascii_alphanumeric()),
        });
        if !has_content {
            return Err(format!("template level {raw_level:?} is empty"));
        }
        levels.push(Level { parts });
    }
    if levels.is_empty() {
        return Err("template is empty".to_string());
    }
    Ok(levels)
}

fn parse_level(level: &str) -> Result<Vec<TemplatePart>, String> {
    let mut parts = Vec::new();
    let mut chars = level.chars().peekable();
    let mut literal = String::new();
    while let Some(c) = chars.next() {
        if c == '{' {
            if !literal.is_empty() {
                parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
            }
            let mut name = String::new();
            let mut closed = false;
            for nc in chars.by_ref() {
                if nc == '}' {
                    closed = true;
                    break;
                }
                name.push(nc);
            }
            if !closed {
                return Err("unclosed '{' in template".to_string());
            }
            let ph = Placeholder::parse(&name)
                .ok_or_else(|| format!("unknown placeholder {{{name}}}"))?;
            parts.push(TemplatePart::Placeholder(ph));
        } else if c == '}' {
            return Err("unexpected '}' in template".to_string());
        } else {
            literal.push(c);
        }
    }
    if !literal.is_empty() {
        parts.push(TemplatePart::Literal(literal));
    }
    Ok(parts)
}

// ── Validation (§9) ───────────────────────────────────────────────────────────────

impl SegmentationConfig {
    /// Validate the whole config. Errors identify the offending band index / placeholder.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("unsupported version {} (expected 1)", self.version));
        }
        // root_tag must be a valid ltree path, not protected.
        TagPath::parse(&self.root_tag, false).map_err(|e| format!("root_tag: {e}"))?;
        if let Some(ca) = &self.catch_all {
            if ca.name.contains('.') {
                return Err("catch_all.name must be a single ltree label".to_string());
            }
            TagPath::parse(&ca.name, false).map_err(|e| format!("catch_all.name: {e}"))?;
        }
        for (i, band) in self.bands.iter().enumerate() {
            band.validate().map_err(|e| format!("bands[{i}]: {e}"))?;
        }
        Ok(())
    }
}

impl Band {
    fn validate(&self) -> Result<(), String> {
        if let (Some(from), Some(to)) = (self.from, self.to) {
            if from >= to {
                return Err("from must be < to".to_string());
            }
        }
        let levels = parse_template(&self.template)?;
        // Collect the placeholders present.
        let mut present: Vec<(&str, Placeholder)> = Vec::new();
        for level in &levels {
            for part in &level.parts {
                if let TemplatePart::Placeholder(ph) = part {
                    present.push((placeholder_name(*ph), *ph));
                }
            }
        }
        // Every `parts` key must appear in the template.
        for key in self.parts.keys() {
            let ph = Placeholder::parse(key)
                .ok_or_else(|| format!("parts.{key}: unknown placeholder"))?;
            if !present.iter().any(|(_, p)| *p == ph) {
                return Err(format!("parts.{key}: placeholder not used in template"));
            }
        }
        // Validate each part config against its placeholder's capabilities.
        for (name, ph) in &present {
            if let Some(pc) = self.parts.get(*name) {
                validate_part(*ph, name, pc)?;
            }
        }
        if let Some(off) = &self.offset {
            off.validate()?;
        }
        Ok(())
    }
}

fn validate_part(ph: Placeholder, name: &str, pc: &PartConfig) -> Result<(), String> {
    if let Some(stride) = pc.stride {
        if stride < 1 {
            return Err(format!("parts.{name}.stride: must be ≥ 1"));
        }
    }
    if let Some(fmt) = &pc.format {
        let strided = pc.stride.map(|s| s > 1).unwrap_or(false);
        if fmt.numeric == Some(false) && !ph.has_named_form() {
            return Err(format!(
                "parts.{name}: field '{name}' has no named form for numeric: false"
            ));
        }
        if fmt.abbrev == Some(true) && !ph.has_named_form() {
            return Err(format!(
                "parts.{name}: field '{name}' has no named form for abbrev"
            ));
        }
        if let Some(pad) = fmt.pad {
            if pad < 1 {
                return Err(format!("parts.{name}.format.pad: must be ≥ 1"));
            }
        }
        if (fmt.bound.is_some() && fmt.bound != Some(Bound::Start) || fmt.range_sep.is_some())
            && !strided
        {
            return Err(format!(
                "parts.{name}.format: bound / range_sep only meaningful with stride > 1"
            ));
        }
    }
    Ok(())
}

impl Offset {
    fn validate(&self) -> Result<(), String> {
        for (label, v) in [
            ("months", self.months),
            ("days", self.days),
            ("hours", self.hours),
            ("minutes", self.minutes),
        ] {
            if let Some(v) = v {
                if v < 0 {
                    return Err(format!("offset.{label}: must be ≥ 0"));
                }
            }
        }
        Ok(())
    }
}

// ── Resolution (§7) ─────────────────────────────────────────────────────────────

impl SegmentationConfig {
    /// Resolve the single segment tag for a picture, or `None`. Pure (§7). Assumes a validated
    /// config (an unparseable template / placeholder yields `None` defensively).
    pub fn resolve(&self, captured_at: Option<NaiveDateTime>) -> Option<TagPath> {
        let Some(t) = captured_at else {
            // Undated: catch-all only when it opts in.
            return self
                .catch_all
                .as_ref()
                .filter(|ca| ca.include_undated)
                .map(|ca| self.catch_all_tag(ca));
        };

        for band in &self.bands {
            if band.enabled && band.contains(t) {
                return band.render(t, self.hemisphere).map(|rendered| {
                    TagPath::from_ltree(format!("{}.{}", self.root_tag, rendered))
                });
            }
        }
        self.catch_all.as_ref().map(|ca| self.catch_all_tag(ca))
    }

    fn catch_all_tag(&self, ca: &CatchAll) -> TagPath {
        TagPath::from_ltree(format!("{}.{}", self.root_tag, ca.name))
    }
}

impl Band {
    fn contains(&self, t: NaiveDateTime) -> bool {
        let d = t.date();
        self.from.map(|f| d >= f).unwrap_or(true) && self.to.map(|to| d < to).unwrap_or(true)
    }

    /// Render this band's template for `t`, returning the path **relative to root_tag** (levels
    /// joined with `.`, each sanitized). `None` on a malformed template (defensive).
    fn render(&self, t: NaiveDateTime, hemisphere: Hemisphere) -> Option<String> {
        let shifted = self.offset.map(|o| o.apply(t)).unwrap_or(t);
        let levels = parse_template(&self.template).ok()?;
        let mut out_levels = Vec::with_capacity(levels.len());
        for level in &levels {
            let mut rendered = String::new();
            for part in &level.parts {
                match part {
                    TemplatePart::Literal(s) => rendered.push_str(s),
                    TemplatePart::Placeholder(ph) => {
                        let name = placeholder_name(*ph);
                        let pc = self.parts.get(name);
                        rendered.push_str(&render_placeholder(*ph, shifted, hemisphere, pc));
                    }
                }
            }
            let label = TagPath::slugify_label(&rendered);
            out_levels.push(label);
        }
        Some(out_levels.join("."))
    }
}

impl Offset {
    /// Subtract the offset from `t` (moves the boundary later).
    fn apply(&self, t: NaiveDateTime) -> NaiveDateTime {
        let mut t = t;
        if let Some(m) = self.months.filter(|m| *m > 0) {
            t = t.checked_sub_months(Months::new(m as u32)).unwrap_or(t);
        }
        if let Some(d) = self.days.filter(|d| *d > 0) {
            t = t.checked_sub_days(Days::new(d as u64)).unwrap_or(t);
        }
        let mins = self.hours.unwrap_or(0) * 60 + self.minutes.unwrap_or(0);
        if mins != 0 {
            t -= chrono::Duration::minutes(mins);
        }
        t
    }
}

// ── Placeholder projection + rendering (§4.1, §5) ─────────────────────────────────

fn placeholder_name(ph: Placeholder) -> &'static str {
    match ph {
        Placeholder::Year => "year",
        Placeholder::IsoYear => "iso_year",
        Placeholder::Quarter => "quarter",
        Placeholder::Season => "season",
        Placeholder::Month => "month",
        Placeholder::Week => "week",
        Placeholder::Day => "day",
        Placeholder::Weekday => "weekday",
        Placeholder::Daypart => "daypart",
    }
}

/// Render one placeholder for `t`, applying `stride` and `format`.
fn render_placeholder(
    ph: Placeholder,
    t: NaiveDateTime,
    hemisphere: Hemisphere,
    pc: Option<&PartConfig>,
) -> String {
    let raw = projected_value(ph, t, hemisphere);
    let stride = pc.and_then(|p| p.stride).unwrap_or(1).max(1);
    let fmt = pc.and_then(|p| p.format.as_ref());

    // Block start/end for strided fields, in the field's natural origin/cycle.
    let (origin, cycle_end) = field_range(ph, t); // inclusive [origin, cycle_end]
    let block_start = origin + ((raw - origin) / stride as i64) * stride as i64;

    let numeric = fmt
        .and_then(|f| f.numeric)
        .unwrap_or(ph.numeric_by_default());

    if stride > 1 && numeric {
        let bound = fmt.and_then(|f| f.bound).unwrap_or(Bound::Start);
        let inclusive_end = fmt.and_then(|f| f.inclusive_end).unwrap_or(false);
        let sep = fmt
            .and_then(|f| f.range_sep.clone())
            .unwrap_or_else(|| "_".to_string());
        let raw_end = block_start + stride as i64;
        let end_val = if inclusive_end {
            (raw_end - 1).min(cycle_end)
        } else {
            raw_end
        };
        return match bound {
            Bound::Start => fmt_num(block_start, fmt),
            Bound::End => fmt_num(end_val, fmt),
            Bound::Range => format!(
                "{}{sep}{}",
                fmt_num(block_start, fmt),
                fmt_num(end_val, fmt)
            ),
        };
    }

    // Non-strided (or named): a strided named field renders the block-start value's name.
    let value = if stride > 1 { block_start } else { raw };
    if numeric {
        fmt_num(value, fmt)
    } else {
        fmt_name(ph, value, fmt)
    }
}

/// The placeholder's integer value at `t` (after offset), before stride/format.
fn projected_value(ph: Placeholder, t: NaiveDateTime, hemisphere: Hemisphere) -> i64 {
    match ph {
        Placeholder::Year => t.year() as i64,
        Placeholder::IsoYear => t.iso_week().year() as i64,
        Placeholder::Quarter => ((t.month() - 1) / 3 + 1) as i64,
        Placeholder::Season => season_index(t.month(), hemisphere) as i64,
        Placeholder::Month => t.month() as i64,
        Placeholder::Week => t.iso_week().week() as i64,
        Placeholder::Day => t.day() as i64,
        Placeholder::Weekday => t.weekday().number_from_monday() as i64,
        Placeholder::Daypart => (t.hour() / 6 + 1) as i64,
    }
}

/// Inclusive natural range `[origin, cycle_end]` of a field, used for stride block alignment and
/// the inclusive-end clamp. `year`/`iso_year` have origin 0 and an effectively-open cycle.
fn field_range(ph: Placeholder, t: NaiveDateTime) -> (i64, i64) {
    match ph {
        Placeholder::Year | Placeholder::IsoYear => (0, i64::MAX / 4),
        Placeholder::Quarter => (1, 4),
        Placeholder::Season => (1, 4),
        Placeholder::Month => (1, 12),
        Placeholder::Week => (1, last_iso_week(t)),
        Placeholder::Day => (1, last_day_of_month(t) as i64),
        Placeholder::Weekday => (1, 7),
        Placeholder::Daypart => (1, 4),
    }
}

fn last_day_of_month(t: NaiveDateTime) -> u32 {
    let (y, m) = (t.year(), t.month());
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let first_next = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    (first_next - Days::new(1)).day()
}

fn last_iso_week(t: NaiveDateTime) -> i64 {
    // The ISO week of Dec 28 is always the last week of its ISO year.
    let dec28 = NaiveDate::from_ymd_opt(t.iso_week().year(), 12, 28).unwrap();
    dec28.iso_week().week() as i64
}

/// 1 = Winter, 2 = Spring, 3 = Summer, 4 = Autumn (northern); southern shifts by two seasons.
fn season_index(month: u32, hemisphere: Hemisphere) -> u32 {
    let north = match month {
        12 | 1 | 2 => 1,
        3..=5 => 2,
        6..=8 => 3,
        _ => 4,
    };
    match hemisphere {
        Hemisphere::North => north,
        Hemisphere::South => (north + 1) % 4 + 1,
    }
}

fn fmt_num(value: i64, fmt: Option<&PartFormat>) -> String {
    let pad = fmt.and_then(|f| f.pad).unwrap_or(0) as usize;
    if pad > 0 {
        format!("{value:0pad$}")
    } else {
        value.to_string()
    }
}

fn fmt_name(ph: Placeholder, value: i64, fmt: Option<&PartFormat>) -> String {
    let abbrev = fmt.and_then(|f| f.abbrev).unwrap_or(false);
    let case = fmt.and_then(|f| f.case).unwrap_or(Case::Pascal);
    let name = name_for(ph, value, abbrev);
    apply_case(name, case)
}

fn name_for(ph: Placeholder, value: i64, abbrev: bool) -> String {
    let full: &[&str] = match ph {
        Placeholder::Season => &["Winter", "Spring", "Summer", "Autumn"],
        Placeholder::Month => &[
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        Placeholder::Weekday => &[
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
        Placeholder::Daypart => &["Night", "Morning", "Afternoon", "Evening"],
        _ => return value.to_string(),
    };
    let idx = (value - 1).clamp(0, full.len() as i64 - 1) as usize;
    let word = full[idx];
    if abbrev {
        word.chars().take(3).collect()
    } else {
        word.to_string()
    }
}

fn apply_case(s: String, case: Case) -> String {
    match case {
        Case::Pascal => s, // names are already PascalCase
        Case::Lower => s.to_lowercase(),
        Case::Upper => s.to_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn cfg(v: serde_json::Value) -> SegmentationConfig {
        let c: SegmentationConfig = serde_json::from_value(v).unwrap();
        c.validate().unwrap();
        c
    }

    fn resolve(c: &SegmentationConfig, s: &str) -> Option<String> {
        c.resolve(Some(dt(s))).map(|t| t.as_ltree().to_string())
    }

    #[test]
    fn worked_example_from_spec() {
        let c = cfg(json!({
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
        }));
        assert_eq!(
            resolve(&c, "1994-06-10 10:00:00").as_deref(),
            Some("Photos.Travel.1990s")
        );
        assert_eq!(
            resolve(&c, "2018-03-02 10:00:00").as_deref(),
            Some("Photos.Travel.2018")
        );
        assert_eq!(
            resolve(&c, "2015-03-20 10:00:00").as_deref(),
            Some("Photos.Travel.2015.March")
        );
        assert_eq!(
            resolve(&c, "2015-10-04 10:00:00").as_deref(),
            Some("Photos.Travel.School_year_15_16")
        );
        assert_eq!(
            resolve(&c, "2021-07-15 10:00:00").as_deref(),
            Some("Photos.Travel.Summer_2021")
        );
        assert_eq!(
            c.resolve(None).map(|t| t.as_ltree().to_string()).as_deref(),
            Some("Photos.Travel.Undated")
        );
    }

    #[test]
    fn first_covering_band_wins() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [
                { "from": "2020-01-01", "to": "2021-01-01", "template": "Override" },
                { "from": null, "to": null, "template": "{year}" }
            ]
        }));
        assert_eq!(
            resolve(&c, "2020-06-01 00:00:00").as_deref(),
            Some("Y.Override")
        );
        assert_eq!(
            resolve(&c, "2019-06-01 00:00:00").as_deref(),
            Some("Y.2019")
        );
    }

    #[test]
    fn half_open_range() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": "2020-01-01", "to": "2021-01-01", "template": "In" }]
        }));
        assert_eq!(resolve(&c, "2020-12-31 23:59:59").as_deref(), Some("Y.In"));
        assert_eq!(resolve(&c, "2021-01-01 00:00:00"), None); // `to` is exclusive
    }

    #[test]
    fn no_catch_all_means_no_tag() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": "2020-01-01", "to": "2021-01-01", "template": "In" }]
        }));
        assert_eq!(resolve(&c, "2019-01-01 00:00:00"), None);
        assert_eq!(c.resolve(None), None);
    }

    #[test]
    fn undated_only_with_include_undated() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "catch_all": { "name": "Unsorted", "include_undated": false },
            "bands": []
        }));
        assert_eq!(c.resolve(None), None);
        // A dated picture matching no band still lands in the catch-all.
        assert_eq!(
            resolve(&c, "2019-01-01 00:00:00").as_deref(),
            Some("Y.Unsorted")
        );
    }

    #[test]
    fn disabled_band_is_skipped() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [
                { "from": null, "to": null, "enabled": false, "template": "Off" },
                { "from": null, "to": null, "template": "{year}" }
            ]
        }));
        assert_eq!(
            resolve(&c, "2020-06-01 00:00:00").as_deref(),
            Some("Y.2020")
        );
    }

    #[test]
    fn decade_stride_and_range() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}",
                        "parts": { "year": { "stride": 5, "format": { "bound": "range", "range_sep": "_" } } } }]
        }));
        assert_eq!(
            resolve(&c, "2022-06-01 00:00:00").as_deref(),
            Some("Y.2020_2025")
        );
        let c2 = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}",
                        "parts": { "year": { "stride": 5, "format": { "bound": "range", "inclusive_end": true } } } }]
        }));
        assert_eq!(
            resolve(&c2, "2022-06-01 00:00:00").as_deref(),
            Some("Y.2020_2024")
        );
    }

    #[test]
    fn day_stride_resets_each_month() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}.{month}.D{day}",
                        "parts": { "day": { "stride": 5 } } }]
        }));
        // Feb 27 → block starts at 26 (short last block).
        assert_eq!(
            resolve(&c, "2024-02-27 00:00:00").as_deref(),
            Some("Y.2024.2.D26")
        );
        assert_eq!(
            resolve(&c, "2024-02-03 00:00:00").as_deref(),
            Some("Y.2024.2.D1")
        );
    }

    #[test]
    fn month_padding_and_names() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}.{month}",
                        "parts": { "month": { "format": { "pad": 2 } } } }]
        }));
        assert_eq!(
            resolve(&c, "2024-08-01 00:00:00").as_deref(),
            Some("Y.2024.08")
        );

        let abbr = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{month}",
                        "parts": { "month": { "format": { "numeric": false, "abbrev": true } } } }]
        }));
        assert_eq!(
            resolve(&abbr, "2024-08-01 00:00:00").as_deref(),
            Some("Y.Aug")
        );

        let lower = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{month}",
                        "parts": { "month": { "format": { "numeric": false, "case": "lower" } } } }]
        }));
        assert_eq!(
            resolve(&lower, "2024-08-01 00:00:00").as_deref(),
            Some("Y.august")
        );
    }

    #[test]
    fn offset_shifts_photographic_day() {
        // 4am day boundary: a 02:00 capture renders under the previous day.
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}.{month}.{day}",
                        "offset": { "hours": 4 } }]
        }));
        assert_eq!(
            resolve(&c, "2024-08-15 02:00:00").as_deref(),
            Some("Y.2024.8.14")
        );
        assert_eq!(
            resolve(&c, "2024-08-15 06:00:00").as_deref(),
            Some("Y.2024.8.15")
        );
    }

    #[test]
    fn daypart_and_weekday_names() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{weekday}.{daypart}" }]
        }));
        // 2024-08-15 is a Thursday, 20:00 = Evening.
        assert_eq!(
            resolve(&c, "2024-08-15 20:00:00").as_deref(),
            Some("Y.Thursday.Evening")
        );
    }

    #[test]
    fn iso_year_week_boundary() {
        // 2024-12-30 is ISO week 1 of ISO-year 2025.
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{iso_year}.W{week}",
                        "parts": { "week": { "format": { "pad": 2 } } } }]
        }));
        assert_eq!(
            resolve(&c, "2024-12-30 00:00:00").as_deref(),
            Some("Y.2025.W01")
        );
    }

    #[test]
    fn validation_rejects_bad_configs() {
        // numeric:false on a numeric-only field.
        let bad: SegmentationConfig = serde_json::from_value(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}",
                        "parts": { "year": { "format": { "numeric": false } } } }]
        }))
        .unwrap();
        assert!(bad.validate().is_err());

        // parts key not in template.
        let bad2: SegmentationConfig = serde_json::from_value(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{year}",
                        "parts": { "month": { "stride": 2 } } }]
        }))
        .unwrap();
        assert!(bad2.validate().is_err());

        // from >= to.
        let bad3: SegmentationConfig = serde_json::from_value(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": "2021-01-01", "to": "2020-01-01", "template": "{year}" }]
        }))
        .unwrap();
        assert!(bad3.validate().is_err());

        // protected root_tag.
        let bad4: SegmentationConfig = serde_json::from_value(json!({
            "version": 1, "root_tag": "SharedToMe.X", "bands": []
        }))
        .unwrap();
        assert!(bad4.validate().is_err());

        // unknown placeholder.
        let bad5: SegmentationConfig = serde_json::from_value(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "{nope}" }]
        }))
        .unwrap();
        assert!(bad5.validate().is_err());
    }

    #[test]
    fn sanitization_collapses_invalid_chars() {
        let c = cfg(json!({
            "version": 1, "root_tag": "Y",
            "bands": [{ "from": null, "to": null, "template": "Summer {year}!",
                        "parts": {} }]
        }));
        // The literal space + '!' collapse to a single '_' / trimmed.
        assert_eq!(
            resolve(&c, "2024-08-01 00:00:00").as_deref(),
            Some("Y.Summer_2024")
        );
    }
}
