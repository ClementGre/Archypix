//! Video metadata + frame extraction via the `ffprobe`/`ffmpeg` binaries.
//!
//! Videos don't carry EXIF; their metadata lives in container atoms/tags. `ffprobe` reads them as
//! JSON (capture date, GPS, rotation, make/model, duration, codecs) which we map onto the same
//! [`ExtractedExif`]/[`FullExif`] shape used for images. A still frame grabbed by `ffmpeg` feeds the
//! existing WebP thumbnail pipeline, so videos get the three thumbnail variants like images.
//!
//! Both functions shell out and block — call inside `tokio::task::spawn_blocking`.

use crate::error::{Result, WorkerError};
use archypix_common::job::{CameraExif, ExtractedExif, FullExif};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tracing::{debug, instrument, warn};

/// Run `ffprobe` and map the container metadata onto [`ExtractedExif`].
///
/// `orientation` is deliberately left `None`: the frame grabbed by [`extract_frame`] is auto-rotated
/// upright (matching inline playback), so the thumbnail needs no display rotation.
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn extract_video_metadata(path: &Path) -> Result<ExtractedExif> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|e| {
            WorkerError::Exif(format!(
                "failed to spawn ffprobe (installed / on PATH?): {e}"
            ))
        })?;
    if !out.status.success() {
        return Err(WorkerError::Exif(format!(
            "ffprobe exited with {}",
            out.status
        )));
    }
    let json: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| WorkerError::Exif(format!("ffprobe JSON parse: {e}")))?;

    let format_tags = json.get("format").and_then(|f| f.get("tags"));
    let streams = json
        .get("streams")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let video_stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(Value::as_str) == Some("video"));
    let audio_stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(Value::as_str) == Some("audio"));
    // Apple/Android cameras attach geo + make/model as stream-level tags too — search both.
    let tag = |key: &str| -> Option<String> {
        tag_in(format_tags, key).or_else(|| video_stream.and_then(|s| tag_in(s.get("tags"), key)))
    };

    // Capture time as local wall clock (matching image EXIF, which carries no zone). Apple's
    // `creationdate` embeds the offset, so parse it directly. A bare `creation_time` is UTC; Samsung
    // (and some others) record the local offset in a separate `*.utc_offset` tag — apply it so the
    // stored time is local, not 2 h off. Without an offset tag we keep the UTC wall clock.
    let captured_at = tag("com.apple.quicktime.creationdate")
        .as_deref()
        .and_then(parse_video_datetime)
        .or_else(|| {
            let ts = tag("creation_time").or_else(|| tag("date"))?;
            let offset = utc_offset_tag(format_tags, video_stream);
            parse_utc_to_local(&ts, offset.as_deref())
        });

    let (gps_lat, gps_lng, gps_alt) = tag("com.apple.quicktime.location.ISO6709")
        .or_else(|| tag("location"))
        .or_else(|| tag("location-eng"))
        .as_deref()
        .and_then(parse_iso6709)
        .map(|(la, lo, al)| (Some(la), Some(lo), al))
        .unwrap_or((None, None, None));

    let duration_s = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .map(round2);

    let camera = CameraExif {
        // Android rarely embeds make/model; fall back to a generic label from `com.android.version`
        // (e.g. "Android 16 Device") so the brand isn't blank for those clips.
        camera_brand: tag("com.apple.quicktime.make")
            .or_else(|| tag("make"))
            .or_else(|| tag("com.android.version").map(|v| android_brand(&v))),
        camera_model: tag("com.apple.quicktime.model").or_else(|| tag("model")),
        duration_s,
        video_codec: video_stream
            .and_then(|s| s.get("codec_name").and_then(Value::as_str))
            .map(str::to_string),
        audio_codec: audio_stream
            .and_then(|s| s.get("codec_name").and_then(Value::as_str))
            .map(str::to_string),
        frame_rate: video_stream
            .and_then(|s| s.get("avg_frame_rate").and_then(Value::as_str))
            .and_then(parse_frame_rate),
        ..Default::default()
    };

    // Raw stream dimensions — a fallback; the decoded frame is authoritative when thumbnailed.
    let width = video_stream
        .and_then(|s| s.get("width").and_then(Value::as_i64))
        .map(|n| n as i32);
    let height = video_stream
        .and_then(|s| s.get("height").and_then(Value::as_i64))
        .map(|n| n as i32);

    debug!(
        captured_at = ?captured_at,
        has_gps = gps_lat.is_some(),
        duration_s = ?duration_s,
        "video metadata extraction complete"
    );

    Ok(ExtractedExif {
        width,
        height,
        exif: FullExif {
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation: None,
            camera,
        },
    })
}

/// Whether both `ffmpeg` and `ffprobe` are runnable (installed and on `PATH`). Logged once at worker
/// startup so a missing-binary deployment surfaces immediately, not as a per-video error.
pub fn tools_available() -> bool {
    ["ffprobe", "ffmpeg"].iter().all(|bin| {
        Command::new(bin)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Grab one upright still frame from the video into `dest` (PNG), for the thumbnail pipeline.
///
/// Seeks ~1 s in for a representative frame; falls back to the first frame for very short clips.
#[instrument(skip(src, dest), fields(file = ?src.file_name()))]
pub fn extract_frame(src: &Path, dest: &Path) -> Result<()> {
    for seek in ["1", "0"] {
        let status = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-ss", seek])
            .arg("-i")
            .arg(src)
            .args(["-frames:v", "1", "-an", "-f", "image2"])
            .arg(dest)
            .status()
            .map_err(|e| {
                WorkerError::Imaging(format!(
                    "failed to spawn ffmpeg (installed / on PATH?): {e}"
                ))
            })?;
        if status.success() && dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            debug!(dest = %dest.display(), seek, "video frame extracted");
            return Ok(());
        }
        let _ = std::fs::remove_file(dest);
    }
    Err(WorkerError::Imaging("ffmpeg produced no frame".to_string()))
}

/// Look up a tag on a `tags` JSON object case-insensitively (ffprobe casing varies by container).
fn tag_in(tags: Option<&Value>, key: &str) -> Option<String> {
    let obj = tags?.as_object()?;
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Generic device label for Android clips that omit make/model but carry `com.android.version`
/// (e.g. `"Android 16 Device"`).
fn android_brand(version: &str) -> String {
    format!("Android {version} Device")
}

/// The local UTC-offset tag a device records alongside a UTC `creation_time` — vendor-prefixed
/// (e.g. `com.samsung.android.utc_offset`), so match any key ending in `utc_offset`. Format then
/// video-stream tags.
fn utc_offset_tag(format_tags: Option<&Value>, video: Option<&Value>) -> Option<String> {
    let find = |tags: Option<&Value>| -> Option<String> {
        tags?
            .as_object()?
            .iter()
            .find(|(k, _)| k.to_ascii_lowercase().ends_with("utc_offset"))
            .and_then(|(_, v)| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    find(format_tags).or_else(|| find(video.and_then(|s| s.get("tags"))))
}

/// Parse a UTC `creation_time` into a local-wall-clock `NaiveDateTime` using `utc_offset` when given
/// (e.g. `+0200` / `+02:00`); otherwise fall back to the plain (UTC-wall-clock) parse.
fn parse_utc_to_local(ts: &str, utc_offset: Option<&str>) -> Option<NaiveDateTime> {
    if let Some(off) = utc_offset.and_then(parse_fixed_offset) {
        if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
            return Some(dt.with_timezone(&off).naive_local());
        }
    }
    parse_video_datetime(ts)
}

/// Parse a numeric UTC offset string (`+0200`, `+02:00`, `-0530`) into a `FixedOffset`.
fn parse_fixed_offset(s: &str) -> Option<FixedOffset> {
    let s = s.trim();
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    let digits = rest.replace(':', "");
    if digits.len() < 4 {
        return None;
    }
    let h: i32 = digits.get(0..2)?.parse().ok()?;
    let m: i32 = digits.get(2..4)?.parse().ok()?;
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

/// Parse a video capture timestamp into a naive local wall-clock time (matching image EXIF, which
/// carries no zone). A value with an offset (`com.apple.quicktime.creationdate`) keeps its local
/// wall clock; a bare `Z`/UTC `creation_time` is taken as-is.
fn parse_video_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    // Offset forms: `+02:00` (rfc3339 / `%:z`) and `+0200` (Apple QuickTime / `%z`).
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%dT%H:%M:%S%.f%z"] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.naive_local());
        }
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y:%m:%d %H:%M:%S",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), fmt) {
            return Some(dt);
        }
    }
    None
}

/// Parse an ISO 6709 location string (`+45.1234-006.8765+1200.000/`) into `(lat, lng, alt?)`.
fn parse_iso6709(s: &str) -> Option<(f64, f64, Option<i32>)> {
    let s = s.trim().trim_end_matches('/');
    // Signed decimal runs: lat, lng, optional altitude — split on the sign that starts each.
    let mut nums: Vec<f64> = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    for i in 1..=bytes.len() {
        let at_sign = i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-');
        if at_sign || i == bytes.len() {
            if let Ok(v) = s[start..i].parse::<f64>() {
                nums.push(v);
            }
            start = i;
        }
    }
    match nums.as_slice() {
        [lat, lng, rest @ ..] => Some((*lat, *lng, rest.first().map(|a| *a as i32))),
        _ => None,
    }
}

/// Parse an ffprobe `avg_frame_rate` rational (`"30000/1001"`) into fps.
fn parse_frame_rate(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let (num, den): (f64, f64) = (num.parse().ok()?, den.parse().ok()?);
    if den == 0.0 {
        return None;
    }
    Some(round2(num / den))
}

fn round2(f: f64) -> f64 {
    (f * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso6709_lat_lng_alt() {
        assert_eq!(
            parse_iso6709("+45.1234-006.8765+1200.000/"),
            Some((45.1234, -6.8765, Some(1200)))
        );
    }

    #[test]
    fn iso6709_lat_lng_only() {
        assert_eq!(
            parse_iso6709("+48.8583+002.2945/"),
            Some((48.8583, 2.2945, None))
        );
    }

    #[test]
    fn frame_rate_rational() {
        assert_eq!(parse_frame_rate("30000/1001"), Some(29.97));
        assert_eq!(parse_frame_rate("25/1"), Some(25.0));
        assert_eq!(parse_frame_rate("0/0"), None);
    }

    #[test]
    fn datetime_with_offset_keeps_local() {
        assert_eq!(
            parse_video_datetime("2024-08-03T14:30:00+0200"),
            NaiveDateTime::parse_from_str("2024-08-03 14:30:00", "%Y-%m-%d %H:%M:%S").ok()
        );
    }

    #[test]
    fn datetime_utc_z() {
        assert_eq!(
            parse_video_datetime("2024-08-03T12:30:00.000000Z"),
            NaiveDateTime::parse_from_str("2024-08-03 12:30:00", "%Y-%m-%d %H:%M:%S").ok()
        );
    }

    #[test]
    fn fixed_offset_forms() {
        assert_eq!(parse_fixed_offset("+0200"), FixedOffset::east_opt(2 * 3600));
        assert_eq!(
            parse_fixed_offset("+02:00"),
            FixedOffset::east_opt(2 * 3600)
        );
        assert_eq!(
            parse_fixed_offset("-0530"),
            FixedOffset::east_opt(-(5 * 3600 + 30 * 60))
        );
        assert_eq!(parse_fixed_offset("0200"), None);
    }

    #[test]
    fn samsung_utc_creation_time_shifts_to_local() {
        // S26 sample: UTC creation_time + com.samsung.android.utc_offset → local wall clock (20:47).
        assert_eq!(
            parse_utc_to_local("2026-06-12T18:47:10.000000Z", Some("+0200")),
            NaiveDateTime::parse_from_str("2026-06-12 20:47:10", "%Y-%m-%d %H:%M:%S").ok()
        );
    }

    #[test]
    fn utc_creation_time_without_offset_stays_utc() {
        assert_eq!(
            parse_utc_to_local("2026-06-12T18:47:10.000000Z", None),
            NaiveDateTime::parse_from_str("2026-06-12 18:47:10", "%Y-%m-%d %H:%M:%S").ok()
        );
    }

    #[test]
    fn android_brand_label() {
        assert_eq!(android_brand("16"), "Android 16 Device");
    }

    #[test]
    fn utc_offset_tag_matches_vendor_prefixed_key() {
        let tags = serde_json::json!({
            "creation_time": "2026-06-12T18:47:10.000000Z",
            "com.samsung.android.utc_offset": "+0200"
        });
        assert_eq!(utc_offset_tag(Some(&tags), None).as_deref(), Some("+0200"));
    }

    /// End-to-end against a generated clip: real ffprobe metadata + ffmpeg frame-grab. Skips when
    /// ffmpeg is not installed (so the suite still passes outside `nix develop`).
    #[test]
    fn extract_from_generated_clip() {
        if !tools_available() {
            eprintln!("ffmpeg/ffprobe not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("clip.mp4");
        let ok = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=3:size=320x240:rate=25",
                "-metadata",
                "creation_time=2024-08-03T14:30:00.000000Z",
                "-metadata",
                "location=+45.1234-006.8765/",
            ])
            .arg(&video)
            .status()
            .unwrap()
            .success();
        assert!(ok, "ffmpeg failed to generate the test clip");

        let meta = extract_video_metadata(&video).unwrap();
        assert_eq!(meta.width, Some(320));
        assert_eq!(meta.height, Some(240));
        assert_eq!(meta.exif.gps_lat, Some(45.1234));
        assert_eq!(meta.exif.gps_lng, Some(-6.8765));
        assert_eq!(meta.exif.camera.video_codec.as_deref(), Some("h264"));
        assert_eq!(meta.exif.camera.frame_rate, Some(25.0));
        assert_eq!(meta.exif.camera.duration_s, Some(3.0));
        assert!(meta.exif.captured_at.is_some());

        let frame = dir.path().join("frame.png");
        extract_frame(&video, &frame).unwrap();
        assert!(frame.metadata().unwrap().len() > 0);
    }
}
