//! Picture metadata (feature 37): ExifTool for the tags, the HEIF container for orientation.
//! Blocking — call inside `tokio::task::spawn_blocking`.

mod bmff;
mod exiftool;

use crate::error::Result;
use archypix_common::job::{ExtractedExif, FullExif};
use bmff::Heif;
use std::path::Path;
use tracing::{debug, instrument};

pub use exiftool::exiftool_available;

/// Read a file's metadata (feature 33 §3.2). A HEIF's orientation comes from its container (37 §2).
#[instrument(skip(path), fields(file = ?path.file_name()))]
pub fn read(path: &Path) -> Result<ExtractedExif> {
    let mut extracted = exiftool::read(path)?;
    let heif = Heif::open(path)?;
    if let Some(heif) = &heif {
        extracted.exif.orientation = heif.orientation();
    }
    debug!(
        captured_at = ?extracted.exif.captured_at,
        has_gps = extracted.exif.gps_lat.is_some(),
        orientation = ?extracted.exif.orientation,
        heif = heif.is_some(),
        "metadata read"
    );
    Ok(extracted)
}

/// Rewrite the file's editable metadata to exactly `target` (feature 31 §3.3).
#[instrument(skip(path, target), fields(file = ?path.file_name()))]
pub fn write_target(path: &Path, target: &FullExif) -> Result<()> {
    exiftool::write_target(path, target)?;
    let heif = Heif::open(path)?;
    let is_heif = heif.is_some();
    if let Some(heif) = heif {
        heif.write_orientation(path, target.orientation)?;
    }
    debug!(heif = is_heif, "metadata written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archypix_common::job::CameraExif;
    use chrono::NaiveDateTime;
    use std::path::PathBuf;
    use std::process::Command;

    /// Smallest valid JPEG the fixtures stamp EXIF onto (8x8 grey, no metadata).
    const TINY_JPEG: &[u8] = b"\
    \xff\xd8\xff\xe0\x00\x10\x4a\x46\x49\x46\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xdb\x00\x43\
    \x00\x28\x1c\x1e\x23\x1e\x19\x28\x23\x21\x23\x2d\x2b\x28\x30\x3c\x64\x41\x3c\x37\x37\x3c\x7b\x58\
    \x5d\x49\x64\x91\x80\x99\x96\x8f\x80\x8c\x8a\xa0\xb4\xe6\xc3\xa0\xaa\xda\xad\x8a\x8c\xc8\xff\xcb\
    \xda\xee\xf5\xff\xff\xff\x9b\xc1\xff\xff\xff\xfa\xff\xe6\xfd\xff\xf8\xff\xc0\x00\x0b\x08\x00\x08\
    \x00\x08\x01\x01\x11\x00\xff\xc4\x00\x14\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
    \x00\x00\x00\x00\xff\xc4\x00\x14\x10\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
    \x00\x00\xff\xda\x00\x08\x01\x01\x00\x00\x3f\x00\x3f\xff\xd9";
    const PLAIN_HEIC: &[u8] = include_bytes!("metadata/testdata/plain.heic");
    const APPLE6_HEIC: &[u8] = include_bytes!("metadata/testdata/apple6.heic");

    /// Write `bytes` into `dir` and stamp `tags` onto them with exiftool.
    fn fixture(dir: &Path, name: &str, bytes: &[u8], tags: &[&str]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        if !tags.is_empty() {
            let status = Command::new("exiftool")
                .arg("-q")
                .arg("-overwrite_original")
                .args(tags)
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
        }
        path
    }

    fn at(s: &str) -> Option<NaiveDateTime> {
        Some(NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap())
    }

    /// Equality up to the GPS tolerance of the diff badges (31 §8): degrees are reassembled from the
    /// stored deg/min/sec rationals, so the last ULPs differ.
    fn assert_same(actual: &FullExif, expected: &FullExif, what: &str) {
        let close = |x: Option<f64>, y: Option<f64>| match (x, y) {
            (Some(x), Some(y)) => (x - y).abs() <= 1e-5,
            (x, y) => x == y,
        };
        assert!(
            close(actual.gps_lat, expected.gps_lat),
            "{what}: gps_lat {actual:?}"
        );
        assert!(
            close(actual.gps_lng, expected.gps_lng),
            "{what}: gps_lng {actual:?}"
        );
        let strip = |e: &FullExif| FullExif {
            gps_lat: None,
            gps_lng: None,
            ..e.clone()
        };
        assert_eq!(strip(actual), strip(expected), "{what}");
    }

    /// What rexiv2 read from each file before it was removed (37 §2): the normalization points
    /// (33 §3.3) stay pinned.
    #[test]
    fn the_corpus_reads_as_rexiv2_did() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let corpus: &[(&str, &[&str], FullExif)] = &[
            ("bare.jpg", &[], FullExif::default()),
            (
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
                    "-GPSHPositioningError=4.735",
                    "-Make=Canon",
                    "-Model=EOS R5",
                    "-ISO=400",
                    "-FocalLengthIn35mmFormat=50",
                    "-FNumber=1.8",
                    "-ExposureTime=1/125",
                ],
                FullExif {
                    captured_at: at("2024-06-01 12:00:00"),
                    gps_lat: Some(48.858222),
                    gps_lng: Some(2.2945),
                    gps_alt: Some(35),
                    gps_accuracy_m: Some(4.74),
                    orientation: Some(6),
                    camera: CameraExif {
                        camera_brand: Some("Canon".into()),
                        camera_model: Some("EOS R5".into()),
                        focal_length_mm: Some(50.0),
                        f_number: Some(1.8),
                        iso_speed: Some(400),
                        exposure_time_num: Some(1),
                        exposure_time_den: Some(125),
                        ..Default::default()
                    },
                },
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
                FullExif {
                    gps_lat: Some(-33.8688),
                    gps_lng: Some(-151.2093),
                    gps_alt: Some(-12),
                    camera: CameraExif {
                        exposure_time_num: Some(1),
                        exposure_time_den: Some(4000),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ),
            (
                // Coordinates with no altitude at all: the presence gate holds.
                "no_altitude.jpg",
                &[
                    "-GPSLatitude=10.5",
                    "-GPSLatitudeRef=N",
                    "-GPSLongitude=20.25",
                    "-GPSLongitudeRef=E",
                    "-Orientation#=1",
                ],
                FullExif {
                    gps_lat: Some(10.5),
                    gps_lng: Some(20.25),
                    orientation: Some(1),
                    ..Default::default()
                },
            ),
            (
                // The capture-date chain falls through to IFD0:ModifyDate; 9 is no orientation.
                "modify_date_only.jpg",
                &["-ModifyDate=2019:03:04 05:06:07", "-Orientation#=9"],
                FullExif {
                    captured_at: at("2019-03-04 05:06:07"),
                    ..Default::default()
                },
            ),
            (
                // A receiver with no lock stamps a complete (0,0) group: no location (30 §12.11).
                "no_fix.jpg",
                &[
                    "-GPSLatitude=0",
                    "-GPSLatitudeRef=N",
                    "-GPSLongitude=0",
                    "-GPSLongitudeRef=E",
                    "-GPSAltitude=0",
                    "-GPSAltitudeRef#=0",
                    "-GPSHPositioningError=0",
                    "-DateTimeOriginal=2024:06:01 12:00:00",
                ],
                FullExif {
                    captured_at: at("2024-06-01 12:00:00"),
                    ..Default::default()
                },
            ),
        ];
        for (name, tags, expected) in corpus {
            let read = read(&fixture(dir.path(), name, TINY_JPEG, tags)).unwrap();
            assert_same(&read.exif, expected, name);
        }
    }

    /// Writing a target then reading it back yields the target, on both write paths: every field
    /// set, a sparse target (each absent field deleted), and an empty one.
    #[test]
    fn a_target_round_trips_exactly() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let full = FullExif {
            captured_at: at("2024-06-01 12:00:00"),
            gps_lat: Some(-48.858222),
            gps_lng: Some(2.2945),
            gps_alt: Some(-35),
            gps_accuracy_m: Some(1234.56),
            orientation: Some(5),
            camera: CameraExif {
                camera_brand: Some("Canon".into()),
                camera_model: Some("EOS R5".into()),
                focal_length_mm: Some(50.0),
                f_number: Some(1.78),
                iso_speed: Some(400),
                exposure_time_num: Some(1),
                exposure_time_den: Some(125),
                ..Default::default()
            },
        };
        let sparse = FullExif {
            gps_lat: Some(10.5),
            gps_lng: Some(-20.25),
            orientation: Some(1),
            camera: CameraExif {
                iso_speed: Some(100),
                ..Default::default()
            },
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in [("a.jpg", TINY_JPEG), ("a.heic", PLAIN_HEIC)] {
            let path = fixture(dir.path(), name, bytes, &[]);
            for target in [&full, &sparse, &FullExif::default()] {
                write_target(&path, target).unwrap();
                assert_same(&read(&path).unwrap().exif, target, name);
            }
        }
    }

    /// A date edit leaves no other copy of the old date: ImageIO prefers a stale IPTC date over
    /// EXIF once the IPTC digest mismatches (37 §3.3).
    #[test]
    fn a_date_edit_leaves_no_stale_copy() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(
            dir.path(),
            "a.jpg",
            TINY_JPEG,
            &[
                "-EXIF:DateTimeOriginal=2024:10:14 22:49:36",
                "-XMP-xmp:CreateDate=2024:10:14 22:49:36",
                "-XMP-photoshop:DateCreated=2024:10:14 22:49:36",
                "-XMP-exif:DateTimeOriginal=2024:10:14 22:49:36",
                "-XMP-exif:DateTimeDigitized=2024:10:14 22:49:36",
                "-XMP-tiff:DateTime=2024:10:14 22:49:36",
                "-IPTC:DateCreated=2024:10:14",
                "-IPTC:TimeCreated=22:49:36",
                "-IPTC:DigitalCreationDate=2024:10:14",
                "-IPTC:DigitalCreationTime=22:49:36",
            ],
        );
        let target = FullExif {
            captured_at: at("2026-09-17 21:08:00"),
            ..Default::default()
        };
        write_target(&path, &target).unwrap();

        let dates = Command::new("exiftool")
            .args(["-a", "-s3", "-time:all", "-x", "System:all"])
            .arg(&path)
            .output()
            .unwrap();
        let dates = String::from_utf8(dates.stdout).unwrap();
        assert!(
            !dates.contains("2024") && !dates.contains("22:49:36"),
            "{dates}"
        );
        assert!(dates.contains("2026:09:17 21:08:00"), "{dates}");
    }

    /// Viewers ignore EXIF `Orientation` in a HEIF, so the read does too (37 §2).
    #[test]
    fn a_heif_orientation_is_its_containers() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let exif_only = fixture(dir.path(), "exif.heic", PLAIN_HEIC, &["-Orientation#=6"]);
        assert_eq!(read(&exif_only).unwrap().exif.orientation, None);
        let imageio = fixture(
            dir.path(),
            "imageio.heic",
            APPLE6_HEIC,
            &["-Orientation#=1"],
        );
        assert_eq!(read(&imageio).unwrap().exif.orientation, Some(6));
    }

    /// A newline in a value must not reach exiftool as a separate argument.
    #[test]
    fn a_value_cannot_inject_arguments() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), "a.jpg", TINY_JPEG, &[]);
        let target = FullExif {
            camera: CameraExif {
                camera_model: Some("EOS\n-Make=Evil".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        write_target(&path, &target).unwrap();
        let camera = read(&path).unwrap().exif.camera;
        assert_eq!(camera.camera_model.as_deref(), Some("EOS -Make=Evil"));
        assert_eq!(camera.camera_brand, None);
    }

    /// A target that drops the accuracy removes a stale one without touching the location, and a
    /// location-less target keeps it (feature 36 §3).
    #[test]
    fn gps_accuracy_is_its_own_tag() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), "a.jpg", TINY_JPEG, &[]);
        let located = FullExif {
            gps_lat: Some(48.858222),
            gps_lng: Some(2.2945),
            gps_accuracy_m: Some(40.0),
            ..Default::default()
        };
        write_target(&path, &located).unwrap();
        write_target(
            &path,
            &FullExif {
                gps_accuracy_m: None,
                ..located.clone()
            },
        )
        .unwrap();
        let read_back = read(&path).unwrap().exif;
        assert_eq!(read_back.gps_accuracy_m, None);
        assert!(read_back.gps_lat.is_some());

        let bare = FullExif {
            gps_accuracy_m: Some(40.0),
            ..Default::default()
        };
        write_target(&path, &bare).unwrap();
        assert_same(&read(&path).unwrap().exif, &bare, "accuracy only");
    }

    #[test]
    fn a_file_without_metadata_reads_empty() {
        if !exiftool_available() {
            eprintln!("exiftool not found; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), "note.txt", b"not an image at all\n", &[]);
        assert_eq!(read(&path).unwrap().exif, FullExif::default());
    }

    #[test]
    fn an_empty_download_is_an_io_fault_not_a_format_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path(), "truncated.jpg", b"", &[]);
        let err = read(&path).expect_err("an empty file cannot be read");
        assert!(err.is_retriable(), "{err} must be retriable");
        assert_eq!(err.exif_verdict(), None);
    }
}
