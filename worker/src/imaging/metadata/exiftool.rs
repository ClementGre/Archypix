//! The ExifTool engine (feature 37): a pool of stay-open processes. Every call blocks.

use crate::error::{Result, WorkerError};
use archypix_common::job::{CameraExif, ExtractedExif, FullExif, drop_null_island};
use chrono::NaiveDateTime;
use exiftool::{ExifTool, ExifToolError};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, PoisonError};

/// Idle processes; the pool grows to peak concurrency (feature 37 §3.1).
static POOL: Mutex<Vec<ExifTool>> = Mutex::new(Vec::new());

/// How the crate reports a child that died mid-call.
const DIED: &str = "Process terminated unexpectedly.";

/// Run `call` on an idle process (or a new one); it goes back to the pool unless it faulted.
fn with_exiftool<T>(
    call: impl FnOnce(&ExifTool) -> std::result::Result<T, ExifToolError>,
) -> std::result::Result<T, ExifToolError> {
    let idle = POOL.lock().unwrap_or_else(PoisonError::into_inner).pop();
    let tool = match idle {
        Some(tool) => tool,
        None => ExifTool::new()?,
    };
    let out = call(&tool);
    if !matches!(&out, Err(e) if is_tool_fault(e)) {
        POOL.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(tool);
    }
    out
}

/// A failure of the process rather than a verdict about the file.
fn is_tool_fault(e: &ExifToolError) -> bool {
    match e {
        ExifToolError::ExifToolNotFound(_)
        | ExifToolError::Io(_)
        | ExifToolError::ProcessTerminated
        | ExifToolError::StderrDisconnected
        | ExifToolError::MutexPoison(_) => true,
        ExifToolError::ExifToolProcess { message, .. } => message == DIED,
        _ => false,
    }
}

/// Tool faults are retriable whatever the call (feature 31 §6); anything else gets `verdict`.
fn classify(e: ExifToolError, verdict: impl FnOnce(ExifToolError) -> WorkerError) -> WorkerError {
    match e {
        ExifToolError::Io(io) => WorkerError::Io(io),
        e if is_tool_fault(&e) => WorkerError::ToolUnavailable(format!("exiftool: {e}")),
        e => verdict(e),
    }
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

// ── Read (feature 33 §3.2) ───────────────────────────────────────────────────

/// `-a` keeps same-named tags from different IFDs (the capture-date chain), `-g1` groups them so each
/// lookup names the exact IFD, and a trailing `#` forces the numeric value (33 §3.3). `ExposureTime`
/// deliberately keeps its print form, which is the stored fraction.
const READ_ARGS: &[&str] = &[
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
    "-GPSHPositioningError#",
    "-DateTimeOriginal",
    "-CreateDate",
    "-ModifyDate",
    "-ImageWidth",
    "-ImageHeight",
];

pub fn read(path: &Path) -> Result<ExtractedExif> {
    let json = with_exiftool(|tool| tool.json(path, READ_ARGS))
        .map_err(|e| classify(e, |e| classify_open_failure(path, &e)))?;
    Ok(map_json(&json))
}

/// An unreadable or empty file is an IO fault (retriable); a readable one ExifTool rejects is a
/// format verdict (feature 33 §7).
fn classify_open_failure(path: &Path, e: &dyn std::fmt::Display) -> WorkerError {
    match std::fs::metadata(path) {
        Err(io) => WorkerError::Io(io),
        Ok(m) if m.len() == 0 => WorkerError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "file is empty",
        )),
        Ok(_) => match std::fs::File::open(path) {
            Err(io) => WorkerError::Io(io),
            Ok(_) => WorkerError::UnsupportedFormat(format!("exiftool read failed: {e}")),
        },
    }
}

/// Map one `-a -g1` JSON object onto [`ExtractedExif`] (33 §3.3), rounded as edits are (37 §3.2).
fn map_json(json: &Value) -> ExtractedExif {
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
    let mut gps_lat = tag_f64(json, "GPS", "GPSLatitude")
        .map(|v| sign(v, tag_string(json, "GPS", "GPSLatitudeRef"), "S"));
    let mut gps_lng = tag_f64(json, "GPS", "GPSLongitude")
        .map(|v| sign(v, tag_string(json, "GPS", "GPSLongitudeRef"), "W"));
    // Presence-gated, and signed by the ref (1 = below sea level).
    let mut gps_alt = tag_f64(json, "GPS", "GPSAltitude").map(|alt| {
        let below = tag_f64(json, "GPS", "GPSAltitudeRef").is_some_and(|r| r == 1.0);
        let alt = if below { -alt.abs() } else { alt };
        alt as i32
    });
    let mut gps_accuracy_m = tag_f64(json, "GPS", "GPSHPositioningError");
    drop_null_island(
        &mut gps_lat,
        &mut gps_lng,
        &mut gps_alt,
        &mut gps_accuracy_m,
    );

    let orientation = match tag_f64(json, "IFD0", "Orientation") {
        Some(n) if (1.0..=8.0).contains(&n) => Some(n as i16),
        _ => None,
    };

    let (exposure_time_num, exposure_time_den) = tag_string(json, "ExifIFD", "ExposureTime")
        .as_deref()
        .and_then(parse_exposure_time)
        .map_or((None, None), |(n, d)| (Some(n), Some(d)));

    let mut exif = FullExif {
        captured_at,
        gps_lat,
        gps_lng,
        gps_alt,
        gps_accuracy_m,
        orientation,
        camera: CameraExif {
            camera_brand: tag_string(json, "IFD0", "Make").filter(|s| !s.is_empty()),
            camera_model: tag_string(json, "IFD0", "Model").filter(|s| !s.is_empty()),
            focal_length_mm: tag_f64(json, "ExifIFD", "FocalLengthIn35mmFormat"),
            f_number: tag_f64(json, "ExifIFD", "FNumber"),
            iso_speed: tag_f64(json, "ExifIFD", "ISO")
                .or_else(|| tag_f64(json, "XMP-exifEX", "PhotographicSensitivity"))
                .map(|v| v as i32)
                .filter(|&v| v != 0),
            exposure_time_num,
            exposure_time_den,
            ..Default::default()
        },
    };
    exif.normalize();
    ExtractedExif {
        width: tag_f64(json, "File", "ImageWidth").map(|v| v as i32),
        height: tag_f64(json, "File", "ImageHeight").map(|v| v as i32),
        exif,
    }
}

/// Parse a raw EXIF capture timestamp (`"YYYY:MM:DD HH:MM:SS"`, or RFC3339).
fn parse_exif_datetime(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .ok()
}

/// ExifTool's print form for `ExposureTime` is the stored fraction (`"1/125"`) below ~1/4 s and a
/// one-decimal number above it. Both are parsed exactly; a float is never rationalized (33 §3.3).
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

/// The value of `tag` inside family-1 group `group` of an `-g1` JSON object.
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

// ── Write (feature 31 §3.3) ──────────────────────────────────────────────────

/// Rewrite the file's editable EXIF to exactly `target`: every `Some` set, every `None` deleted.
pub fn write_target(path: &Path, target: &FullExif) -> Result<()> {
    let camera = &target.camera;
    let date = target
        .captured_at
        .map(|dt| dt.format("%Y:%m:%d %H:%M:%S").to_string());
    let exposure =
        (camera.exposure_time_num.is_some() || camera.exposure_time_den.is_some()).then(|| {
            let num = camera.exposure_time_num.unwrap_or(0);
            let den = camera.exposure_time_den.unwrap_or(1).max(1);
            format!("{num}/{den}")
        });
    let tags = [
        // Untagged names also update same-named XMP copies; the differently named ones are dropped
        // so no reader prefers a stale date (feature 37 §3.3).
        ("DateTimeOriginal", date.clone()),
        ("CreateDate", date.clone()),
        ("ModifyDate", date),
        ("XMP-photoshop:DateCreated", None),
        ("XMP-exif:DateTimeDigitized", None),
        ("XMP-tiff:DateTime", None),
        ("IPTC:DateCreated", None),
        ("IPTC:TimeCreated", None),
        ("IPTC:DigitalCreationDate", None),
        ("IPTC:DigitalCreationTime", None),
        // `#` writes the numeric value: exiftool silently drops a print-form Orientation.
        ("Orientation#", target.orientation.map(|o| o.to_string())),
        ("GPSLatitude", target.gps_lat.map(|v| v.abs().to_string())),
        (
            "GPSLatitudeRef",
            target.gps_lat.map(|v| hemisphere(v, "N", "S")),
        ),
        ("GPSLongitude", target.gps_lng.map(|v| v.abs().to_string())),
        (
            "GPSLongitudeRef",
            target.gps_lng.map(|v| hemisphere(v, "E", "W")),
        ),
        ("GPSAltitude", target.gps_alt.map(|v| v.abs().to_string())),
        (
            "GPSAltitudeRef#",
            target.gps_alt.map(|v| u8::from(v < 0).to_string()),
        ),
        (
            "GPSHPositioningError",
            target.gps_accuracy_m.map(|v| v.to_string()),
        ),
        ("Make", camera.camera_brand.as_deref().map(text)),
        ("Model", camera.camera_model.as_deref().map(text)),
        ("ISO", camera.iso_speed.map(|v| v.to_string())),
        (
            "FocalLengthIn35mmFormat",
            camera
                .focal_length_mm
                .map(|v| (v.round() as i32).to_string()),
        ),
        ("FNumber", camera.f_number.map(|v| v.to_string())),
        ("ExposureTime", exposure),
    ];

    let mut args = vec!["-overwrite_original".to_string()];
    args.extend(
        tags.into_iter()
            .map(|(tag, value)| format!("-{tag}={}", value.unwrap_or_default())),
    );
    if camera.iso_speed.is_none() {
        // The XMP twin the reader falls back to.
        args.push("-PhotographicSensitivity=".to_string());
    }
    args.push(path.display().to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    with_exiftool(|tool| tool.execute_raw(&args)).map_err(|e| {
        classify(e, |e| {
            WorkerError::Exif(format!("exiftool write failed: {e}"))
        })
    })?;
    Ok(())
}

fn hemisphere(v: f64, positive: &str, negative: &str) -> String {
    if v >= 0.0 { positive } else { negative }.to_string()
}

/// Stay-open arguments are newline-separated: a control character in a value would inject options.
fn text(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archypix_common::transfer::ExifExtraction;

    #[test]
    fn exposure_time_print_forms_parse_exactly() {
        assert_eq!(parse_exposure_time("1/125"), Some((1, 125)));
        assert_eq!(parse_exposure_time("0.5"), Some((1, 2)));
        assert_eq!(parse_exposure_time("2"), Some((2, 1)));
        assert_eq!(parse_exposure_time("30"), Some((30, 1)));
        assert_eq!(parse_exposure_time("1/0"), None);
        assert_eq!(parse_exposure_time("undef"), None);
    }

    #[test]
    fn a_dead_or_missing_process_stays_retriable() {
        let path = Path::new("/nonexistent/x.jpg");
        for e in [
            ExifToolError::ProcessTerminated,
            ExifToolError::StderrDisconnected,
            ExifToolError::MutexPoison("poisoned".into()),
            ExifToolError::ExifToolProcess {
                message: DIED.into(),
                std_err: String::new(),
                command_args: String::new(),
            },
        ] {
            let mapped = classify(e, |e| classify_open_failure(path, &e));
            assert!(mapped.is_retriable(), "{mapped} must not be a file verdict");
            assert_eq!(mapped.exif_verdict(), None);
        }
    }

    /// ExifTool refuses very little (it reports "no metadata" far more often than an error), so the
    /// `Failed` outcome hangs entirely on this branch: a readable file the process rejected.
    #[test]
    fn a_file_exiftool_rejects_is_a_terminal_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.unknownext");
        std::fs::write(&path, [0x37u8; 512]).unwrap();

        let err = classify(
            ExifToolError::ExifToolProcess {
                message: "Error: Unknown file type".into(),
                std_err: "Error: Unknown file type".into(),
                command_args: String::new(),
            },
            |e| classify_open_failure(&path, &e),
        );
        assert_eq!(err.exif_verdict(), Some(ExifExtraction::Failed));
        assert!(!err.is_retriable());
    }

    #[test]
    fn control_characters_cannot_split_an_argument() {
        assert_eq!(text("Canon\n-o\r\n/tmp/x\0"), "Canon -o  /tmp/x ");
    }
}
