use crate::error::{Result, WorkerError};
use archypix_common::job::{CameraExif, ExifField, ExtractedExif, FullExif};
use chrono::NaiveDateTime;
use exiftool::{ExifTool, ExifToolError};
use num_rational::Ratio;
use rexiv2::{GpsInfo, Metadata};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use tracing::{debug, instrument};

static EXIFTOOL: OnceLock<std::result::Result<ExifTool, String>> = OnceLock::new();

const BMFF_EXIF_WRITE_MIMES: &[&str] = &["image/heic", "image/heif", "image/avif"];

/// Read a file's metadata through the engine this MIME dispatches to (feature 33 §3.1).
///
/// BMFF containers go straight to ExifTool (the engine that also writes them); every other format
/// tries rexiv2 and falls back to ExifTool for a second opinion **only** on a format verdict — an
/// IO or tool fault is propagated as-is, since turning it into a file verdict is the bug this
/// dispatch exists to remove.
///
/// Must be called inside `tokio::task::spawn_blocking`: both engines are synchronous.
#[instrument(skip(path), fields(file = ?path.file_name(), exiftool = use_exiftool_for_read(mime_type)))]
pub fn read_metadata(path: &Path, mime_type: Option<&str>) -> Result<ExtractedExif> {
    if use_exiftool_for_read(mime_type) {
        return exiftool_read(path);
    }
    match rexiv2_read(path) {
        Err(WorkerError::UnsupportedFormat(e)) => {
            debug!(error = %e, "rexiv2 could not open the file; retrying with exiftool");
            exiftool_read(path)
        }
        other => other,
    }
}

/// Whether this MIME reads through ExifTool. Same predicate as [`use_exiftool_for_write`], so a
/// format class is read and written by the same engine.
pub fn use_exiftool_for_read(mime_type: Option<&str>) -> bool {
    use_exiftool_for_write(mime_type)
}

/// Load and extract EXIF data from an image file with rexiv2 (GExiv2).
///
/// Must be called inside `tokio::task::spawn_blocking` since rexiv2 is synchronous.
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn rexiv2_read(path: &Path) -> Result<ExtractedExif> {
    let metadata = Metadata::new_from_path(path).map_err(|e| classify_open_failure(path, &e))?;

    let captured_at = extract_first_tag(
        &metadata,
        &[
            "Exif.Photo.DateTimeOriginal",
            "Exif.Photo.DateTimeDigitized",
            "Exif.Image.DateTime",
            "Exif.Image.DateTimeOriginal",
            "Exif.Image.DateTimeDigitized",
        ],
    );

    let gps = metadata.get_gps_info();
    let gps_lat = gps.as_ref().map(|g| g.latitude);
    let gps_lng = gps.as_ref().map(|g| g.longitude);
    // Altitude is only present when the tag exists; lat/lng without altitude is common.
    let gps_alt = gps
        .as_ref()
        .filter(|_| metadata.has_tag("Exif.GPSInfo.GPSAltitude"))
        .map(|g| g.altitude as i32);

    let orientation = match metadata.get_tag_numeric("Exif.Image.Orientation") {
        n @ 1..=8 => Some(n as i16),
        _ => None,
    };

    let width = metadata.get_pixel_width();
    let height = metadata.get_pixel_height();

    // Remaining EXIF fields stored as JSON blob.
    let mut exif_map = serde_json::Map::new();

    if let Ok(brand) = metadata.get_tag_string("Exif.Image.Make") {
        if !brand.is_empty() {
            exif_map.insert("camera_brand".to_string(), serde_json::Value::String(brand));
        }
    }
    if let Ok(model) = metadata.get_tag_string("Exif.Image.Model") {
        if !model.is_empty() {
            exif_map.insert("camera_model".to_string(), serde_json::Value::String(model));
        }
    }
    if let Some(f) = rational_to_f64(metadata.get_tag_rational("Exif.Photo.FocalLengthIn35mmFilm"))
    {
        exif_map.insert("focal_length_mm".to_string(), serde_json::json!(round2(f)));
    }
    if let Some(f) = rational_to_f64(metadata.get_tag_rational("Exif.Photo.FNumber")) {
        exif_map.insert("f_number".to_string(), serde_json::json!(round1(f)));
    }
    if let Some(iso) = extract_iso(&metadata) {
        exif_map.insert("iso_speed".to_string(), serde_json::json!(iso));
    }
    if let Some(et) = metadata.get_tag_rational("Exif.Photo.ExposureTime") {
        exif_map.insert(
            "exposure_time_num".to_string(),
            serde_json::json!(*et.numer()),
        );
        exif_map.insert(
            "exposure_time_den".to_string(),
            serde_json::json!(*et.denom()),
        );
    }

    // The camera/lens map deserializes straight into the typed CameraExif (unknown keys ignored).
    let camera: CameraExif = if exif_map.is_empty() {
        CameraExif::default()
    } else {
        serde_json::from_value(serde_json::Value::Object(exif_map)).unwrap_or_default()
    };

    let captured_at = captured_at.as_deref().and_then(parse_exif_datetime);

    debug!(
        captured_at = ?captured_at,
        has_gps = gps_lat.is_some(),
        "EXIF extraction complete"
    );

    Ok(ExtractedExif {
        width: if width > 0 { Some(width) } else { None },
        height: if height > 0 { Some(height) } else { None },
        exif: FullExif {
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation,
            camera,
        },
    })
}

/// Parse a raw EXIF capture timestamp (`"YYYY:MM:DD HH:MM:SS"`, or RFC3339) into a `NaiveDateTime`.
fn parse_exif_datetime(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()
}

/// Classify a metadata-library open failure (feature 33 §7): unreadable or empty bytes are an IO
/// fault (retriable), a readable file the library cannot parse is a format verdict.
fn classify_open_failure(path: &Path, e: &dyn std::fmt::Display) -> WorkerError {
    match std::fs::metadata(path) {
        Err(io) => WorkerError::Io(io),
        Ok(m) if m.len() == 0 => WorkerError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "file is empty",
        )),
        Ok(_) => match std::fs::File::open(path) {
            Err(io) => WorkerError::Io(io),
            Ok(_) => WorkerError::UnsupportedFormat(format!("failed to open file for EXIF: {e}")),
        },
    }
}

// ── ExifTool reader (feature 33 §3.2) ────────────────────────────────────────

/// One `exiftool -json` request covering every field the rexiv2 reader maps. `-a` keeps
/// same-named tags from different IFDs (the capture-date chain), `-g1` groups them so each lookup
/// names the exact IFD rexiv2 reads, and a trailing `#` forces the numeric value for the fields
/// whose print form would be a description (`-n` per §3.3). `ExposureTime` deliberately keeps its
/// print form, which is the stored fraction.
const EXIFTOOL_READ_ARGS: &[&str] = &[
    "-a",
    "-g1",
    "-Make",
    "-Model",
    "-ISO#",
    "-XMP-exifEX:PhotographicSensitivity#",
    "-FocalLengthIn35mmFormat#",
    "-FNumber#",
    "-ExposureTime",
    "-Orientation#",
    "-GPSLatitude#",
    "-GPSLatitudeRef#",
    "-GPSLongitude#",
    "-GPSLongitudeRef#",
    "-GPSAltitude#",
    "-GPSAltitudeRef#",
    "-DateTimeOriginal",
    "-CreateDate",
    "-ModifyDate",
    "-ImageWidth",
    "-ImageHeight",
];

/// Read a file's metadata with ExifTool, mapped onto the same [`ExtractedExif`] shape the rexiv2
/// reader produces (feature 33 §3.2–3.3).
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn exiftool_read(path: &Path) -> Result<ExtractedExif> {
    let json = exiftool()?
        .json(path, EXIFTOOL_READ_ARGS)
        .map_err(|e| classify_exiftool_error(path, e))?;
    let extracted = map_exiftool_json(&json);
    debug!(
        captured_at = ?extracted.exif.captured_at,
        has_gps = extracted.exif.gps_lat.is_some(),
        "EXIF extraction complete (exiftool)"
    );
    Ok(extracted)
}

/// A failed ExifTool call is a worker-environment fault (retriable) or a file verdict, never both
/// — see feature 33 §3.1.
fn classify_exiftool_error(path: &Path, e: ExifToolError) -> WorkerError {
    match e {
        ExifToolError::ExifToolNotFound(_)
        | ExifToolError::ProcessTerminated
        | ExifToolError::StderrDisconnected
        | ExifToolError::MutexPoison(_) => {
            WorkerError::ToolUnavailable(format!("exiftool read failed: {e}"))
        }
        ExifToolError::Io(io) => WorkerError::Io(io),
        other => classify_open_failure(path, &format_args!("exiftool read failed: {other}")),
    }
}

/// Map one `-a -g1` ExifTool JSON object onto the rexiv2 reader's output shape.
fn map_exiftool_json(json: &Value) -> ExtractedExif {
    // The same 5-tag priority chain `rexiv2_read` walks, in exiv2 → ExifTool naming.
    let captured_at = [
        ("ExifIFD", "DateTimeOriginal"),
        ("ExifIFD", "CreateDate"),
        ("IFD0", "ModifyDate"),
        ("IFD0", "DateTimeOriginal"),
        ("IFD0", "CreateDate"),
    ]
    .iter()
    .find_map(|(group, tag)| tag_string(json, group, tag))
    .as_deref()
    .and_then(parse_exif_datetime);

    let sign = |v: f64, reference: Option<String>, negative: &str| {
        let below = reference
            .as_deref()
            .is_some_and(|r| r.trim().eq_ignore_ascii_case(negative));
        if below { -v.abs() } else { v.abs() }
    };
    let gps_lat = tag_f64(json, "GPS", "GPSLatitude")
        .map(|v| sign(v, tag_string(json, "GPS", "GPSLatitudeRef"), "S"));
    let gps_lng = tag_f64(json, "GPS", "GPSLongitude")
        .map(|v| sign(v, tag_string(json, "GPS", "GPSLongitudeRef"), "W"));
    // Presence-gated like rexiv2's `has_tag` check, and signed by the ref (1 = below sea level).
    let gps_alt = tag_f64(json, "GPS", "GPSAltitude").map(|alt| {
        let below = tag_f64(json, "GPS", "GPSAltitudeRef").is_some_and(|r| r == 1.0);
        let alt = if below { -alt.abs() } else { alt };
        alt as i32
    });

    let orientation = match tag_f64(json, "IFD0", "Orientation") {
        Some(n) if (1.0..=8.0).contains(&n) => Some(n as i16),
        _ => None,
    };

    let camera = CameraExif {
        camera_brand: tag_string(json, "IFD0", "Make").filter(|s| !s.is_empty()),
        camera_model: tag_string(json, "IFD0", "Model").filter(|s| !s.is_empty()),
        focal_length_mm: tag_f64(json, "ExifIFD", "FocalLengthIn35mmFormat").map(round2),
        f_number: tag_f64(json, "ExifIFD", "FNumber").map(round1),
        iso_speed: tag_f64(json, "ExifIFD", "ISO")
            .or_else(|| tag_f64(json, "XMP-exifEX", "PhotographicSensitivity"))
            .map(|v| v as i32)
            .filter(|&v| v != 0),
        ..Default::default()
    };
    let (exposure_time_num, exposure_time_den) = tag_string(json, "ExifIFD", "ExposureTime")
        .as_deref()
        .and_then(parse_exposure_time)
        .map_or((None, None), |(n, d)| (Some(n), Some(d)));

    ExtractedExif {
        width: tag_f64(json, "File", "ImageWidth").map(|v| v as i32),
        height: tag_f64(json, "File", "ImageHeight").map(|v| v as i32),
        exif: FullExif {
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation,
            camera: CameraExif {
                exposure_time_num,
                exposure_time_den,
                ..camera
            },
        },
    }
}

/// ExifTool's print form for `ExposureTime` is the stored fraction (`"1/125"`) below ~1/4 s and a
/// one-decimal number above it. Both are parsed exactly; a float is never rationalized (§3.3).
fn parse_exposure_time(s: &str) -> Option<(i32, i32)> {
    let s = s.trim();
    if let Some((num, den)) = s.split_once('/') {
        let num: i32 = num.trim().parse().ok()?;
        let den: i32 = den.trim().parse().ok()?;
        return (den != 0).then(|| reduce(num, den));
    }
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let den = 10i32.checked_pow(frac.len() as u32)?;
    let num: i32 = format!("{int}{frac}").parse().ok()?;
    Some(reduce(num, den))
}

fn reduce(num: i32, den: i32) -> (i32, i32) {
    let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let g = a.max(1) as i32;
    (num / g, den / g)
}

/// The value of `tag` inside family-1 group `group` of an `-g1` ExifTool JSON object.
fn tag_in<'a>(json: &'a Value, group: &str, tag: &str) -> Option<&'a Value> {
    json.get(group)?.get(tag)
}

fn tag_string(json: &Value, group: &str, tag: &str) -> Option<String> {
    match tag_in(json, group, tag)? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn tag_f64(json: &Value, group: &str, tag: &str) -> Option<f64> {
    match tag_in(json, group, tag)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Apply an EXIF edit (`set` writes, `clear` deletes) into the file at `path` (in-place via rexiv2).
///
/// Every editable field is covered so the file converges to the DB row (the source of truth): the
/// promoted columns (date, GPS, orientation) and the camera/lens fields (make, model, focal length,
/// f-number, ISO, exposure time). Fields not named in either `set` or `clear` are left untouched.
/// Must run inside `tokio::task::spawn_blocking`.
#[instrument(skip(path, set, clear), fields(file = ?path.file_name(), clear_fields = clear.len(), bmff_exiftool = use_exiftool_for_write(mime_type)))]
pub fn write_exif_overrides(
    path: &Path,
    set: &FullExif,
    clear: &[ExifField],
    mime_type: Option<&str>,
) -> Result<()> {
    if use_exiftool_for_write(mime_type) {
        return write_exif_overrides_with_exiftool(path, set, clear);
    }
    write_exif_overrides_with_rexiv2(path, set, clear)
}

/// The fields to delete so the file ends up matching `target` exactly.
///
/// GPS (lat/lng/alt) and exposure time (num/den) are **grouped** in both writers — clearing any
/// member deletes the whole group — so they are only cleared when every member is absent from the
/// target. Without this, a target with coordinates but no altitude would write the coordinates and
/// then delete them again.
pub fn target_clear_fields(target: &FullExif) -> Vec<ExifField> {
    let mut clear = Vec::new();
    if target.captured_at.is_none() {
        clear.push(ExifField::CapturedAt);
    }
    if target.gps_lat.is_none() && target.gps_lng.is_none() && target.gps_alt.is_none() {
        clear.push(ExifField::GpsLat);
    }
    if target.orientation.is_none() {
        clear.push(ExifField::Orientation);
    }
    if target.camera.camera_brand.is_none() {
        clear.push(ExifField::CameraBrand);
    }
    if target.camera.camera_model.is_none() {
        clear.push(ExifField::CameraModel);
    }
    if target.camera.focal_length_mm.is_none() {
        clear.push(ExifField::FocalLengthMm);
    }
    if target.camera.f_number.is_none() {
        clear.push(ExifField::FNumber);
    }
    if target.camera.iso_speed.is_none() {
        clear.push(ExifField::IsoSpeed);
    }
    if target.camera.exposure_time_num.is_none() && target.camera.exposure_time_den.is_none() {
        clear.push(ExifField::ExposureTimeNum);
    }
    clear
}

/// Rewrite the file's editable EXIF to exactly match `target` (feature 31 §3.3): every `Some` field
/// is written, every absent one deleted.
pub fn write_exif_target(path: &Path, target: &FullExif, mime_type: Option<&str>) -> Result<()> {
    write_exif_overrides(path, target, &target_clear_fields(target), mime_type)
}

/// Whether this MIME must use ExifTool for writes (BMFF containers).
pub fn use_exiftool_for_write(mime_type: Option<&str>) -> bool {
    let Some(mime_type) = mime_type else {
        return false;
    };
    BMFF_EXIF_WRITE_MIMES.contains(&mime_type.to_ascii_lowercase().as_str())
}

/// Whether `exiftool` is available on PATH.
pub fn exiftool_available() -> bool {
    Command::new("exiftool")
        .arg("-ver")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn exiftool() -> Result<&'static ExifTool> {
    let loaded = EXIFTOOL.get_or_init(|| ExifTool::new().map_err(|e| e.to_string()));
    match loaded {
        Ok(tool) => Ok(tool),
        // Worker-environment fault (binary missing, spawn refused): retriable, not a file verdict.
        Err(e) => Err(WorkerError::ToolUnavailable(format!(
            "failed to initialize exiftool stay-open process: {e}"
        ))),
    }
}

fn write_exif_overrides_with_exiftool(
    path: &Path,
    set: &FullExif,
    clear: &[ExifField],
) -> Result<()> {
    let mut args: Vec<String> = vec!["-overwrite_original".to_string()];

    // ── Set ────────────────────────────────────────────────────────────────────
    if let Some(dt) = set.captured_at {
        let s = dt.format("%Y:%m:%d %H:%M:%S").to_string();
        args.push(format!("-DateTimeOriginal={s}"));
        args.push(format!("-CreateDate={s}"));
        args.push(format!("-ModifyDate={s}"));
    }
    if let Some(orientation) = set.orientation {
        // `#` forces the numeric ValueConv form; without it exiftool rejects the print-form
        // description and silently drops the tag.
        args.push(format!("-Orientation#={orientation}"));
    }
    if let Some(lat) = set.gps_lat {
        args.push(format!("-GPSLatitude={}", lat.abs()));
        args.push(format!(
            "-GPSLatitudeRef={}",
            if lat >= 0.0 { "N" } else { "S" }
        ));
    }
    if let Some(lng) = set.gps_lng {
        args.push(format!("-GPSLongitude={}", lng.abs()));
        args.push(format!(
            "-GPSLongitudeRef={}",
            if lng >= 0.0 { "E" } else { "W" }
        ));
    }
    if let Some(alt) = set.gps_alt {
        args.push(format!("-GPSAltitude={}", alt.abs()));
        args.push(format!("-GPSAltitudeRef#={}", if alt >= 0 { 0 } else { 1 }));
    } else if set.gps_lat.is_some() || set.gps_lng.is_some() {
        // Coordinates without an altitude: drop any altitude the file still carries.
        args.push("-GPSAltitude=".to_string());
        args.push("-GPSAltitudeRef=".to_string());
    }
    if let Some(ref brand) = set.camera.camera_brand {
        args.push(format!("-Make={brand}"));
    }
    if let Some(ref model) = set.camera.camera_model {
        args.push(format!("-Model={model}"));
    }
    if let Some(iso) = set.camera.iso_speed {
        args.push(format!("-ISO={iso}"));
    }
    if let Some(focal) = set.camera.focal_length_mm {
        args.push(format!("-FocalLengthIn35mmFormat={}", focal.round() as i32));
    }
    if let Some(fnum) = set.camera.f_number {
        args.push(format!("-FNumber={fnum}"));
    }
    if set.camera.exposure_time_num.is_some() || set.camera.exposure_time_den.is_some() {
        let num = set.camera.exposure_time_num.unwrap_or(0);
        let den = set.camera.exposure_time_den.unwrap_or(1).max(1);
        args.push(format!("-ExposureTime={num}/{den}"));
    }

    // ── Clear ──────────────────────────────────────────────────────────────────
    let mut clear_gps = false;
    let mut clear_exposure = false;
    for field in clear {
        match field {
            ExifField::CapturedAt => {
                args.push("-DateTimeOriginal=".to_string());
                args.push("-CreateDate=".to_string());
                args.push("-ModifyDate=".to_string());
            }
            ExifField::GpsLat | ExifField::GpsLng | ExifField::GpsAlt => {
                clear_gps = true;
            }
            ExifField::Orientation => {
                args.push("-Orientation=".to_string());
            }
            ExifField::CameraBrand => {
                args.push("-Make=".to_string());
            }
            ExifField::CameraModel => {
                args.push("-Model=".to_string());
            }
            ExifField::FocalLengthMm => {
                args.push("-FocalLengthIn35mmFormat=".to_string());
            }
            ExifField::FNumber => {
                args.push("-FNumber=".to_string());
            }
            ExifField::IsoSpeed => {
                args.push("-ISO=".to_string());
                args.push("-PhotographicSensitivity=".to_string());
            }
            ExifField::ExposureTimeNum | ExifField::ExposureTimeDen => {
                clear_exposure = true;
            }
        }
    }
    if clear_gps {
        args.push("-GPSLatitude=".to_string());
        args.push("-GPSLatitudeRef=".to_string());
        args.push("-GPSLongitude=".to_string());
        args.push("-GPSLongitudeRef=".to_string());
        args.push("-GPSAltitude=".to_string());
        args.push("-GPSAltitudeRef=".to_string());
    }
    if clear_exposure {
        args.push("-ExposureTime=".to_string());
    }

    args.push(path.display().to_string());
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    exiftool()?
        .execute_raw(&arg_refs)
        .map_err(|e| WorkerError::Exif(format!("exiftool write failed: {e}")))?;

    debug!(path = %path.display(), "EXIF overrides written with exiftool");
    Ok(())
}

fn write_exif_overrides_with_rexiv2(
    path: &Path,
    set: &FullExif,
    clear: &[ExifField],
) -> Result<()> {
    let metadata = Metadata::new_from_path(path).map_err(|e| classify_open_failure(path, &e))?;

    // ── Set ────────────────────────────────────────────────────────────────────
    if let Some(dt) = set.captured_at {
        let s = dt.format("%Y:%m:%d %H:%M:%S").to_string();
        // Write all three common date/time tags; ignore per-tag failures (format-dependent).
        for tag in &[
            "Exif.Photo.DateTimeOriginal",
            "Exif.Photo.DateTimeDigitized",
            "Exif.Image.DateTime",
        ] {
            let _ = metadata.set_tag_string(tag, &s);
        }
    }
    if let Some(orientation) = set.orientation {
        let _ = metadata.set_tag_numeric("Exif.Image.Orientation", orientation as i32);
    }
    // GPS: write when at least one coordinate is supplied. `set_gps_info` always writes an
    // altitude, so drop it again when the target has none (the group cannot be cleared piecemeal).
    if set.gps_lat.is_some() || set.gps_lng.is_some() {
        let gps = GpsInfo {
            longitude: set.gps_lng.unwrap_or(0.0),
            latitude: set.gps_lat.unwrap_or(0.0),
            altitude: set.gps_alt.unwrap_or(0) as f64,
        };
        let _ = metadata.set_gps_info(&gps);
        if set.gps_alt.is_none() {
            let _ = metadata.clear_tag("Exif.GPSInfo.GPSAltitude");
            let _ = metadata.clear_tag("Exif.GPSInfo.GPSAltitudeRef");
        }
    }
    if let Some(ref brand) = set.camera.camera_brand {
        let _ = metadata.set_tag_string("Exif.Image.Make", brand);
    }
    if let Some(ref model) = set.camera.camera_model {
        let _ = metadata.set_tag_string("Exif.Image.Model", model);
    }
    if let Some(iso) = set.camera.iso_speed {
        let _ = metadata.set_tag_numeric("Exif.Photo.ISOSpeedRatings", iso);
    }
    if let Some(focal) = set.camera.focal_length_mm {
        let _ = metadata.set_tag_rational(
            "Exif.Photo.FocalLengthIn35mmFilm",
            &Ratio::new((focal * 100.0).round() as i32, 100),
        );
    }
    if let Some(fnum) = set.camera.f_number {
        let _ = metadata.set_tag_rational(
            "Exif.Photo.FNumber",
            &Ratio::new((fnum * 10.0).round() as i32, 10),
        );
    }
    if set.camera.exposure_time_num.is_some() || set.camera.exposure_time_den.is_some() {
        let num = set.camera.exposure_time_num.unwrap_or(0);
        let den = set.camera.exposure_time_den.unwrap_or(1).max(1);
        let _ = metadata.set_tag_rational("Exif.Photo.ExposureTime", &Ratio::new(num, den));
    }

    // ── Clear ──────────────────────────────────────────────────────────────────
    for field in clear {
        match field {
            ExifField::CapturedAt => {
                for tag in &[
                    "Exif.Photo.DateTimeOriginal",
                    "Exif.Photo.DateTimeDigitized",
                    "Exif.Image.DateTime",
                ] {
                    let _ = metadata.clear_tag(tag);
                }
            }
            ExifField::GpsLat | ExifField::GpsLng | ExifField::GpsAlt => {
                metadata.delete_gps_info();
            }
            ExifField::Orientation => {
                let _ = metadata.clear_tag("Exif.Image.Orientation");
            }
            ExifField::CameraBrand => {
                let _ = metadata.clear_tag("Exif.Image.Make");
            }
            ExifField::CameraModel => {
                let _ = metadata.clear_tag("Exif.Image.Model");
            }
            ExifField::FocalLengthMm => {
                let _ = metadata.clear_tag("Exif.Photo.FocalLengthIn35mmFilm");
            }
            ExifField::FNumber => {
                let _ = metadata.clear_tag("Exif.Photo.FNumber");
            }
            ExifField::IsoSpeed => {
                let _ = metadata.clear_tag("Exif.Photo.ISOSpeedRatings");
                let _ = metadata.clear_tag("Exif.Photo.PhotographicSensitivity");
            }
            ExifField::ExposureTimeNum | ExifField::ExposureTimeDen => {
                let _ = metadata.clear_tag("Exif.Photo.ExposureTime");
            }
        }
    }

    metadata
        .save_to_file(path)
        .map_err(|e| WorkerError::Exif(format!("failed to save EXIF overrides: {e}")))?;

    debug!(path = %path.display(), "EXIF overrides written with rexiv2");
    Ok(())
}

fn extract_first_tag(metadata: &Metadata, tags: &[&str]) -> Option<String> {
    for tag in tags {
        if let Ok(val) = metadata.get_tag_string(tag) {
            if !val.trim().is_empty() {
                return Some(val);
            }
        }
    }
    None
}

fn extract_iso(metadata: &Metadata) -> Option<i32> {
    for tag in &[
        "Exif.Photo.ISOSpeedRatings",
        "Exif.Photo.PhotographicSensitivity",
        "Xmp.exifEX.PhotographicSensitivity",
    ] {
        let val = metadata.get_tag_numeric(tag);
        if val != 0 {
            return Some(val);
        }
    }
    None
}

fn rational_to_f64(r: Option<Ratio<i32>>) -> Option<f64> {
    r.and_then(|r| {
        if *r.denom() == 0 {
            None
        } else {
            Some(*r.numer() as f64 / *r.denom() as f64)
        }
    })
}

fn round1(f: f64) -> f64 {
    (f * 10.0).round() / 10.0
}

fn round2(f: f64) -> f64 {
    (f * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use archypix_common::job::CameraExif;
    use archypix_common::transfer::ExifExtraction;

    fn gps_target(lat: Option<f64>, lng: Option<f64>, alt: Option<i32>) -> FullExif {
        FullExif {
            gps_lat: lat,
            gps_lng: lng,
            gps_alt: alt,
            ..Default::default()
        }
    }

    #[test]
    fn coordinates_without_altitude_do_not_clear_gps() {
        let clear = target_clear_fields(&gps_target(Some(48.85), Some(2.35), None));
        assert!(
            !clear
                .iter()
                .any(|f| matches!(f, ExifField::GpsLat | ExifField::GpsLng | ExifField::GpsAlt))
        );
    }

    #[test]
    fn fully_absent_gps_is_cleared() {
        let clear = target_clear_fields(&gps_target(None, None, None));
        assert!(clear.contains(&ExifField::GpsLat));
    }

    #[test]
    fn partial_exposure_time_does_not_clear_the_pair() {
        let target = FullExif {
            camera: CameraExif {
                exposure_time_num: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let clear = target_clear_fields(&target);
        assert!(
            !clear
                .iter()
                .any(|f| matches!(f, ExifField::ExposureTimeNum | ExifField::ExposureTimeDen))
        );
    }

    #[test]
    fn a_full_target_clears_nothing() {
        let target = FullExif {
            captured_at: Some(
                chrono::NaiveDateTime::parse_from_str("2024:06:01 12:00:00", "%Y:%m:%d %H:%M:%S")
                    .unwrap(),
            ),
            gps_lat: Some(48.85),
            gps_lng: Some(2.35),
            gps_alt: Some(35),
            orientation: Some(1),
            camera: CameraExif {
                camera_brand: Some("Canon".into()),
                camera_model: Some("EOS R5".into()),
                focal_length_mm: Some(50.0),
                f_number: Some(1.8),
                iso_speed: Some(400),
                exposure_time_num: Some(1),
                exposure_time_den: Some(200),
                ..Default::default()
            },
        };
        assert!(target_clear_fields(&target).is_empty());
    }

    #[test]
    fn an_empty_target_clears_every_group_once() {
        // date, GPS, orientation, brand, model, focal, f-number, ISO, exposure.
        assert_eq!(target_clear_fields(&FullExif::default()).len(), 9);
    }

    // ── Engine dispatch, fallback and classification (feature 33 §3) ──────────

    #[test]
    fn bmff_mimes_read_through_exiftool() {
        assert!(use_exiftool_for_read(Some("image/HEIC")));
        assert!(use_exiftool_for_read(Some("image/avif")));
        assert!(!use_exiftool_for_read(Some("image/jpeg")));
        assert!(!use_exiftool_for_read(None));
    }

    #[test]
    fn an_exiftool_outage_during_fallback_stays_retriable() {
        let path = Path::new("/nonexistent/x.jpg");
        for e in [
            ExifToolError::ProcessTerminated,
            ExifToolError::StderrDisconnected,
            ExifToolError::MutexPoison("poisoned".into()),
        ] {
            let mapped = classify_exiftool_error(path, e);
            assert!(mapped.is_retriable(), "{mapped} must not be a file verdict");
            assert_eq!(mapped.exif_verdict(), None);
        }
    }

    #[test]
    fn exposure_time_print_forms_parse_exactly() {
        assert_eq!(parse_exposure_time("1/125"), Some((1, 125)));
        assert_eq!(parse_exposure_time("0.5"), Some((1, 2)));
        assert_eq!(parse_exposure_time("2"), Some((2, 1)));
        assert_eq!(parse_exposure_time("30"), Some((30, 1)));
        assert_eq!(parse_exposure_time("1/0"), None);
        assert_eq!(parse_exposure_time("undef"), None);
    }

    // ── Differential parity across a corpus (feature 33 §3.3, §13) ───────────

    /// Smallest valid JPEG the fixtures stamp EXIF onto (8x8 grey, no metadata).
    const TINY_JPEG: &[u8] = b"\
    \xff\xd8\xff\xe0\x00\x10\x4a\x46\x49\x46\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xdb\x00\x43\
    \x00\x28\x1c\x1e\x23\x1e\x19\x28\x23\x21\x23\x2d\x2b\x28\x30\x3c\x64\x41\x3c\x37\x37\x3c\x7b\x58\
    \x5d\x49\x64\x91\x80\x99\x96\x8f\x80\x8c\x8a\xa0\xb4\xe6\xc3\xa0\xaa\xda\xad\x8a\x8c\xc8\xff\xcb\
    \xda\xee\xf5\xff\xff\xff\x9b\xc1\xff\xff\xff\xfa\xff\xe6\xfd\xff\xf8\xff\xc0\x00\x0b\x08\x00\x08\
    \x00\x08\x01\x01\x11\x00\xff\xc4\x00\x14\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
    \x00\x00\x00\x00\xff\xc4\x00\x14\x10\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
    \x00\x00\xff\xda\x00\x08\x01\x01\x00\x00\x3f\x00\x3f\xff\xd9";

    /// Write `TINY_JPEG` into `dir` and stamp `tags` onto it with exiftool.
    fn fixture(dir: &Path, name: &str, tags: &[&str]) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, TINY_JPEG).unwrap();
        let mut args = vec!["-overwrite_original"];
        args.extend_from_slice(tags);
        let display = path.display().to_string();
        args.push(&display);
        exiftool().unwrap().execute_raw(&args).unwrap();
        path
    }

    /// Both engines must map the same file onto the same `FullExif` — §3.3's normalization points
    /// are invisible until they produce permanent, unclearable diff badges. GPS degrees are
    /// compared with the same tolerance the diff badges use (31 §8): the engines reassemble them
    /// from the stored deg/min/sec rationals, so the last ULPs differ.
    fn assert_engines_agree(path: &Path) {
        let a = rexiv2_read(path).expect("rexiv2 read").exif;
        let b = exiftool_read(path).expect("exiftool read").exif;
        let close = |x: Option<f64>, y: Option<f64>| match (x, y) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-5,
            (x, y) => x == y,
        };
        let file = path.display();
        assert!(close(a.gps_lat, b.gps_lat), "gps_lat differs on {file}");
        assert!(close(a.gps_lng, b.gps_lng), "gps_lng differs on {file}");
        let strip = |e: FullExif| FullExif {
            gps_lat: None,
            gps_lng: None,
            ..e
        };
        assert_eq!(strip(a), strip(b), "engines disagree on {file}");
    }

    #[test]
    fn engines_agree_across_the_corpus() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let corpus: &[(&str, &[&str])] = &[
            ("bare.jpg", &[]),
            (
                // Rational exposure, above-sea-level GPS, the full camera block.
                "full.jpg",
                &[
                    "-DateTimeOriginal=2024:06:01 12:00:00",
                    "-CreateDate=2024:06:01 12:00:00",
                    "-ModifyDate=2024:06:01 12:00:01",
                    "-Orientation#=6",
                    "-GPSLatitude=48.858222",
                    "-GPSLatitudeRef=N",
                    "-GPSLongitude=2.2945",
                    "-GPSLongitudeRef=E",
                    "-GPSAltitude=35",
                    "-GPSAltitudeRef#=0",
                    "-Make=Canon",
                    "-Model=EOS R5",
                    "-ISO=400",
                    "-FocalLengthIn35mmFormat=50",
                    "-FNumber=1.8",
                    "-ExposureTime=1/125",
                ],
            ),
            (
                // Southern/western hemisphere and below sea level: both signs are applied.
                "below_sea_level.jpg",
                &[
                    "-GPSLatitude=33.8688",
                    "-GPSLatitudeRef=S",
                    "-GPSLongitude=151.2093",
                    "-GPSLongitudeRef=W",
                    "-GPSAltitude=12",
                    "-GPSAltitudeRef#=1",
                    "-ExposureTime=1/4000",
                ],
            ),
            (
                // Coordinates with no altitude at all: the presence gate must hold on both sides.
                "no_altitude.jpg",
                &[
                    "-GPSLatitude=10.5",
                    "-GPSLatitudeRef=N",
                    "-GPSLongitude=20.25",
                    "-GPSLongitudeRef=E",
                    "-Orientation#=1",
                ],
            ),
            (
                // The capture-date chain falls through to `Exif.Image.DateTime` (IFD0:ModifyDate).
                "modify_date_only.jpg",
                &["-ModifyDate=2019:03:04 05:06:07", "-Orientation#=9"],
            ),
        ];
        for (name, tags) in corpus {
            assert_engines_agree(&fixture(dir.path(), name, tags));
        }
    }

    #[test]
    fn a_file_rexiv2_cannot_open_falls_back_to_exiftool() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, b"not an image at all\n").unwrap();

        assert!(matches!(
            rexiv2_read(&path),
            Err(WorkerError::UnsupportedFormat(_))
        ));
        // The fallback opens it and finds no metadata — a verdict, not a failure.
        let read = read_metadata(&path, Some("text/plain")).expect("fallback read");
        assert_eq!(read.exif, FullExif::default());
    }

    /// ExifTool refuses very little (it reports "no metadata" far more often than an error), so the
    /// `Failed` outcome hangs entirely on this branch: a readable file the process rejected.
    #[test]
    fn a_file_exiftool_rejects_is_a_terminal_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.unknownext");
        std::fs::write(&path, [0x37u8; 512]).unwrap();

        let err = classify_exiftool_error(
            &path,
            ExifToolError::ExifToolProcess {
                message: "Error: Unknown file type".into(),
                std_err: "Error: Unknown file type".into(),
                command_args: String::new(),
            },
        );
        assert_eq!(
            err.exif_verdict(),
            Some(ExifExtraction::Failed),
            "{err} must be a terminal verdict about these bytes"
        );
        assert!(!err.is_retriable());
    }

    /// Regression for the `-Orientation=` print-form bug: exiftool warns
    /// `Can't convert IFD0:Orientation (not in PrintConv)` and drops the tag unless the numeric
    /// ValueConv form (`-Orientation#=`) is used. Exercises the same write path BMFF containers use.
    #[test]
    fn exiftool_write_persists_orientation() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orientation.jpg");
        std::fs::write(&path, TINY_JPEG).unwrap();

        let target = FullExif {
            orientation: Some(6),
            ..Default::default()
        };
        write_exif_overrides_with_exiftool(&path, &target, &target_clear_fields(&target)).unwrap();

        let read = exiftool_read(&path).expect("exiftool read");
        assert_eq!(read.exif.orientation, Some(6));
    }

    #[test]
    fn an_empty_download_is_an_io_fault_not_a_format_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.jpg");
        std::fs::write(&path, b"").unwrap();

        let err = read_metadata(&path, None).expect_err("an empty file cannot be read");
        assert!(err.is_retriable(), "{err} must be retriable");
        assert_eq!(err.exif_verdict(), None);
    }
}
