use crate::error::{Result, WorkerError};
use archypix_common::job::{CameraExif, ExifField, ExtractedExif, FullExif};
use chrono::NaiveDateTime;
use num_rational::Ratio;
use rexiv2::{GpsInfo, Metadata};
use std::path::Path;
use tracing::{debug, instrument};

/// Load and extract EXIF data from an image file.
///
/// Must be called inside `tokio::task::spawn_blocking` since rexiv2 is synchronous.
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn extract_exif(path: &Path) -> Result<ExtractedExif> {
    let metadata = Metadata::new_from_path(path)
        .map_err(|e| WorkerError::Exif(format!("failed to open file for EXIF: {e}")))?;

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
#[instrument(skip(path, set, clear), fields(file = ?path.file_name(), clear_fields = clear.len()))]
pub fn write_exif_overrides(path: &Path, set: &FullExif, clear: &[ExifField]) -> Result<()> {
    let metadata = Metadata::new_from_path(path)
        .map_err(|e| WorkerError::Exif(format!("failed to open file for EXIF write: {e}")))?;

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
    // GPS: write when at least one coordinate is supplied.
    if set.gps_lat.is_some() || set.gps_lng.is_some() {
        let gps = GpsInfo {
            longitude: set.gps_lng.unwrap_or(0.0),
            latitude: set.gps_lat.unwrap_or(0.0),
            altitude: set.gps_alt.unwrap_or(0) as f64,
        };
        let _ = metadata.set_gps_info(&gps);
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

    debug!(path = %path.display(), "EXIF overrides written");
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
