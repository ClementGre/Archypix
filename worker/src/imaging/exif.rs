use crate::error::{Result, WorkerError};
use archypix_common::job::{CameraExif, ExifField, ExtractedExif, FullExif};
use chrono::NaiveDateTime;
use exiftool::ExifTool;
use num_rational::Ratio;
use rexiv2::{GpsInfo, Metadata};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use tracing::{debug, instrument};

static EXIFTOOL: OnceLock<std::result::Result<ExifTool, String>> = OnceLock::new();

const BMFF_EXIF_WRITE_MIMES: &[&str] = &["image/heic", "image/heif", "image/avif"];

/// Load and extract EXIF data from an image file.
///
/// Must be called inside `tokio::task::spawn_blocking` since rexiv2 is synchronous.
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn extract_exif(path: &Path) -> Result<ExtractedExif> {
    let metadata = Metadata::new_from_path(path).map_err(|e| {
        WorkerError::UnsupportedFormat(format!("failed to open file for EXIF: {e}"))
    })?;

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
    static FORCED: OnceLock<std::result::Result<ExifTool, String>> = OnceLock::new();
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
        args.push(format!("-Orientation={orientation}"));
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
        args.push(format!("-GPSAltitudeRef={}", if alt >= 0 { 0 } else { 1 }));
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
    let metadata = Metadata::new_from_path(path).map_err(|e| {
        WorkerError::UnsupportedFormat(format!("failed to open file for EXIF write: {e}"))
    })?;

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
}
