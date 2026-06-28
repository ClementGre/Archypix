use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Enums ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "job_status", rename_all = "lowercase")
)]
pub enum JobStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

/// All job types supported by the worker fleet.
///
/// Implements `FromStr` / `Display` for human-readable string conversion
/// (e.g. query parameters, logs) and optionally `sqlx::Type` when the
/// `sqlx` feature is enabled (back/ only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "job_type", rename_all = "snake_case")
)]
pub enum JobType {
    GenThumbnail,
    MlStyle,
    MlPeople,
    MlGroupLocation,
    EditPicture,
}

impl std::fmt::Display for JobType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::GenThumbnail => "gen_thumbnail",
            Self::MlStyle => "ml_style",
            Self::MlPeople => "ml_people",
            Self::MlGroupLocation => "ml_group_location",
            Self::EditPicture => "edit_picture",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for JobType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gen_thumbnail" => Ok(Self::GenThumbnail),
            "ml_style" => Ok(Self::MlStyle),
            "ml_people" => Ok(Self::MlPeople),
            "ml_group_location" => Ok(Self::MlGroupLocation),
            "edit_picture" => Ok(Self::EditPicture),
            other => Err(format!("unknown job type: '{other}'")),
        }
    }
}

// ── Typed job configs ─────────────────────────────────────────────────────────

/// Discriminated union of all job-specific config payloads.
///
/// Stored as JSONB in the database using an internal `"type"` tag, so the
/// discriminant is self-describing and does not need to be inferred from the
/// `job_type` column.
///
/// ```json
/// {"type": "gen_thumbnail", "picture_id": "…", "is_initial": true}
/// {"type": "edit_picture",  "picture_id": "…", "visual": null}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobConfig {
    GenThumbnail(GenThumbnailConfig),
    EditPicture(EditPictureConfig),
    /// ML jobs carry no extra config for now.
    MlStyle,
    MlPeople,
    MlGroupLocation,
}

impl JobConfig {
    /// Returns the `JobType` discriminant that corresponds to this config variant.
    pub fn job_type(&self) -> JobType {
        match self {
            Self::GenThumbnail(_) => JobType::GenThumbnail,
            Self::EditPicture(_) => JobType::EditPicture,
            Self::MlStyle => JobType::MlStyle,
            Self::MlPeople => JobType::MlPeople,
            Self::MlGroupLocation => JobType::MlGroupLocation,
        }
    }
}

/// Config for `gen_thumbnail` jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenThumbnailConfig {
    pub picture_id: Uuid,
    /// When `true`, this is the first thumbnail generation for this picture:
    /// the worker must also extract and return EXIF metadata.
    pub is_initial: bool,
}

/// Config for `edit_picture` jobs.
///
/// The write-through model makes the DB the source of truth: the backend applies the edit to the
/// `pictures` row synchronously at request time and enqueues this job to reconcile the S3 original's
/// embedded EXIF. The config therefore carries an explicit edit delta plus the revert baseline
/// (`ExifEdit::previous`), so a permanent file-write failure can roll the DB back to the old state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPictureConfig {
    pub picture_id: Uuid,
    /// The EXIF edit delta + revert baseline. `None` for a pure visual job.
    pub exif: Option<ExifEdit>,
    /// Visual pixel-level transformations to apply to the file.
    /// `None` means no visual edits; the original file is unchanged.
    pub visual: Option<VisualTransformations>,
}

impl EditPictureConfig {
    /// Returns `true` when the job requires the worker to generate new thumbnails
    /// (i.e., visual transforms change the image content).
    pub fn needs_thumbnail_regen(&self) -> bool {
        self.visual.is_some()
    }
}

/// An EXIF edit expressed as a `set`/`clear` delta plus the prior full state.
///
/// - `set`: only `Some` fields are written.
/// - `clear`: fields to delete (column → NULL / JSONB key removed / file tag deleted).
/// - `previous`: the full prior value of every editable field, used by the backend's value-gated
///   revert (§4.3) and completion-time convergence (§5). The worker only reads `set`/`clear`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExifEdit {
    pub set: FullExif,
    #[serde(default)]
    pub clear: Vec<ExifField>,
    pub previous: FullExif,
}

impl ExifEdit {
    /// The full snapshot the file/DB reaches once this edit's `set`/`clear` is applied to
    /// `previous`. This is the file's content after a successful reconcile.
    pub fn new_state(&self) -> FullExif {
        self.previous.applied(&self.set, &self.clear)
    }
}

/// One editable EXIF field — the enum form used by `ExifEdit::clear` and the diff machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExifField {
    CapturedAt,
    GpsLat,
    GpsLng,
    GpsAlt,
    Orientation,
    CameraBrand,
    CameraModel,
    FocalLengthMm,
    FNumber,
    IsoSpeed,
    ExposureTimeNum,
    ExposureTimeDen,
}

/// Camera/lens EXIF — the non-promoted editable fields. This is exactly what the `exif_data` JSONB
/// column stores (owned and received rows alike); the five promoted fields live in their own
/// `pictures` columns and in [`FullExif`]. Serialized sparsely (only `Some` keys appear).
///
/// The trailing `video_*`/`audio_*`/`duration_s`/`frame_rate` fields are read-only technical
/// metadata populated for video formats (ffprobe). They are not [`ExifField`]s — never edited, only displayed
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraExif {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub camera_brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub camera_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub focal_length_mm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub f_number: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub iso_speed: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exposure_time_num: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exposure_time_den: Option<i32>,
    /// Media duration in seconds (video/audio).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub duration_s: Option<f64>,
    /// Video codec short name (e.g. `h264`, `hevc`, `vp9`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub video_codec: Option<String>,
    /// Audio codec short name (e.g. `aac`, `opus`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub audio_codec: Option<String>,
    /// Average frame rate (fps) of the primary video stream.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub frame_rate: Option<f64>,
}

/// The full editable EXIF: the five promoted fields (their own `pictures` columns) plus the
/// [`CameraExif`] camera/lens fields. One canonical typed shape for every EXIF carrier — an owner's
/// authoritative snapshot (`remote_exif_data`), a recipient's sticky overrides
/// (`local_exif_overrides`), the announce payload, an edit `set`, and a revert baseline. `Some` =
/// present/written/claimed; `None` = absent/unchanged (context-dependent; explicit removal in an
/// edit is expressed by the paired `clear` list in [`ExifEdit`]). Flattens to one JSON object whose
/// keys are the snake-case [`ExifField`] names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FullExif {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub captured_at: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gps_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gps_lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gps_alt: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub orientation: Option<i16>,
    #[serde(flatten)]
    pub camera: CameraExif,
}

impl FullExif {
    /// Overwrite each field that is `Some` in `set`; leave the rest unchanged. Used for an edit
    /// `set` (`Some` = write) and, via [`Self::merged_with`], for the received-row merge.
    pub fn apply_set(&mut self, set: &FullExif) {
        if set.captured_at.is_some() {
            self.captured_at = set.captured_at;
        }
        if set.gps_lat.is_some() {
            self.gps_lat = set.gps_lat;
        }
        if set.gps_lng.is_some() {
            self.gps_lng = set.gps_lng;
        }
        if set.gps_alt.is_some() {
            self.gps_alt = set.gps_alt;
        }
        if set.orientation.is_some() {
            self.orientation = set.orientation;
        }
        let (c, s) = (&mut self.camera, &set.camera);
        if s.camera_brand.is_some() {
            c.camera_brand = s.camera_brand.clone();
        }
        if s.camera_model.is_some() {
            c.camera_model = s.camera_model.clone();
        }
        if s.focal_length_mm.is_some() {
            c.focal_length_mm = s.focal_length_mm;
        }
        if s.f_number.is_some() {
            c.f_number = s.f_number;
        }
        if s.iso_speed.is_some() {
            c.iso_speed = s.iso_speed;
        }
        if s.exposure_time_num.is_some() {
            c.exposure_time_num = s.exposure_time_num;
        }
        if s.exposure_time_den.is_some() {
            c.exposure_time_den = s.exposure_time_den;
        }
    }

    /// Null a single field.
    pub fn clear_field(&mut self, f: ExifField) {
        match f {
            ExifField::CapturedAt => self.captured_at = None,
            ExifField::GpsLat => self.gps_lat = None,
            ExifField::GpsLng => self.gps_lng = None,
            ExifField::GpsAlt => self.gps_alt = None,
            ExifField::Orientation => self.orientation = None,
            ExifField::CameraBrand => self.camera.camera_brand = None,
            ExifField::CameraModel => self.camera.camera_model = None,
            ExifField::FocalLengthMm => self.camera.focal_length_mm = None,
            ExifField::FNumber => self.camera.f_number = None,
            ExifField::IsoSpeed => self.camera.iso_speed = None,
            ExifField::ExposureTimeNum => self.camera.exposure_time_num = None,
            ExifField::ExposureTimeDen => self.camera.exposure_time_den = None,
        }
    }

    /// Null every field in `fields`.
    pub fn clear_fields(&mut self, fields: &[ExifField]) {
        for f in fields {
            self.clear_field(*f);
        }
    }

    /// `self` with `set` applied (`Some` overwrites) then `clear` nulled — the state the file/DB
    /// reaches after an edit. Used as the revert baseline / convergence comparison.
    pub fn applied(&self, set: &FullExif, clear: &[ExifField]) -> FullExif {
        let mut s = self.clone();
        s.apply_set(set);
        s.clear_fields(clear);
        s
    }

    /// An owner snapshot merged with the recipient's sticky overrides: an overridden (`Some`) field
    /// wins, an un-overridden field flows through from the owner. The materialised effective EXIF.
    pub fn merged_with(&self, overrides: &FullExif) -> FullExif {
        let mut s = self.clone();
        s.apply_set(overrides);
        s
    }

    /// Whether `f` is set (`Some`) here — for MIME preflight / field-presence checks.
    pub fn has(&self, f: ExifField) -> bool {
        match f {
            ExifField::CapturedAt => self.captured_at.is_some(),
            ExifField::GpsLat => self.gps_lat.is_some(),
            ExifField::GpsLng => self.gps_lng.is_some(),
            ExifField::GpsAlt => self.gps_alt.is_some(),
            ExifField::Orientation => self.orientation.is_some(),
            ExifField::CameraBrand => self.camera.camera_brand.is_some(),
            ExifField::CameraModel => self.camera.camera_model.is_some(),
            ExifField::FocalLengthMm => self.camera.focal_length_mm.is_some(),
            ExifField::FNumber => self.camera.f_number.is_some(),
            ExifField::IsoSpeed => self.camera.iso_speed.is_some(),
            ExifField::ExposureTimeNum => self.camera.exposure_time_num.is_some(),
            ExifField::ExposureTimeDen => self.camera.exposure_time_den.is_some(),
        }
    }

    /// The (`set`, `clear`) delta turning `self` into `target`. Empty when already equal.
    pub fn diff_to(&self, target: &FullExif) -> (FullExif, Vec<ExifField>) {
        let mut set = FullExif::default();
        let mut clear = Vec::new();
        macro_rules! diff_p {
            ($field:ident, $variant:ident) => {
                if self.$field != target.$field {
                    match target.$field.clone() {
                        Some(v) => set.$field = Some(v),
                        None => clear.push(ExifField::$variant),
                    }
                }
            };
        }
        macro_rules! diff_c {
            ($field:ident, $variant:ident) => {
                if self.camera.$field != target.camera.$field {
                    match target.camera.$field.clone() {
                        Some(v) => set.camera.$field = Some(v),
                        None => clear.push(ExifField::$variant),
                    }
                }
            };
        }
        diff_p!(captured_at, CapturedAt);
        diff_p!(gps_lat, GpsLat);
        diff_p!(gps_lng, GpsLng);
        diff_p!(gps_alt, GpsAlt);
        diff_p!(orientation, Orientation);
        diff_c!(camera_brand, CameraBrand);
        diff_c!(camera_model, CameraModel);
        diff_c!(focal_length_mm, FocalLengthMm);
        diff_c!(f_number, FNumber);
        diff_c!(iso_speed, IsoSpeed);
        diff_c!(exposure_time_num, ExposureTimeNum);
        diff_c!(exposure_time_den, ExposureTimeDen);
        (set, clear)
    }
}

/// Pixel-level visual transformations to apply to the image file.
///
/// All transforms are optional; at least one must be set for this struct to be
/// useful. The worker applies them in order: crop first, then resize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualTransformations {
    /// Crop the image to a rectangular region before any other transforms.
    pub crop: Option<CropTransform>,
    /// Resize the (optionally cropped) image to fixed dimensions.
    pub resize: Option<ResizeTransform>,
}

/// Crop region in pixels, measured from the top-left corner of the image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropTransform {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Target dimensions for a resize operation. The worker preserves aspect ratio
/// by fitting within the given bounds (no distortion).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResizeTransform {
    pub width: u32,
    pub height: u32,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// Serialize `value` to JSON, deserialize back, re-serialize, and assert the two JSON
    /// strings are identical. `JobConfig` and friends don't derive `PartialEq`, so comparing
    /// JSON is the most reliable equality check.
    fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2, "round-trip must produce identical JSON");
    }

    #[test]
    fn job_config_gen_thumbnail_roundtrips_json() {
        let cfg = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id: Uuid::new_v4(),
            is_initial: true,
        });
        round_trip(&cfg);
    }

    #[test]
    fn job_config_edit_picture_exif_only_roundtrips_json() {
        let cfg = JobConfig::EditPicture(EditPictureConfig {
            picture_id: Uuid::new_v4(),
            exif: Some(ExifEdit {
                set: FullExif {
                    gps_lat: Some(48.8566),
                    gps_lng: Some(2.3522),
                    ..Default::default()
                },
                clear: vec![ExifField::GpsAlt, ExifField::Orientation],
                previous: FullExif {
                    gps_alt: Some(120),
                    orientation: Some(1),
                    ..Default::default()
                },
            }),
            visual: None,
        });
        round_trip(&cfg);
    }

    #[test]
    fn exif_snapshot_applied_and_diff_round_trip() {
        let previous = FullExif {
            gps_lat: Some(1.0),
            gps_alt: Some(50),
            orientation: Some(1),
            ..Default::default()
        };
        let set = FullExif {
            gps_lat: Some(2.0),
            ..Default::default()
        };
        let clear = vec![ExifField::GpsAlt];
        let new_state = previous.applied(&set, &clear);
        assert_eq!(new_state.gps_lat, Some(2.0));
        assert_eq!(new_state.gps_alt, None);
        assert_eq!(new_state.orientation, Some(1));

        // diff from previous to new_state reproduces the delta.
        let (dset, dclear) = previous.diff_to(&new_state);
        assert_eq!(dset.gps_lat, Some(2.0));
        assert_eq!(dclear, vec![ExifField::GpsAlt]);
        // No-op diff when equal.
        let (empty_set, empty_clear) = new_state.diff_to(&new_state);
        assert!(empty_set.gps_lat.is_none() && empty_clear.is_empty());
    }

    #[test]
    fn job_config_edit_picture_visual_roundtrips_json() {
        let cfg = JobConfig::EditPicture(EditPictureConfig {
            picture_id: Uuid::new_v4(),
            exif: None,
            visual: Some(VisualTransformations {
                crop: Some(CropTransform {
                    x: 10,
                    y: 20,
                    width: 800,
                    height: 600,
                }),
                resize: Some(ResizeTransform {
                    width: 1920,
                    height: 1080,
                }),
            }),
        });
        round_trip(&cfg);
    }

    #[test]
    fn job_config_ml_variants_roundtrip_json() {
        round_trip(&JobConfig::MlStyle);
        round_trip(&JobConfig::MlPeople);
        round_trip(&JobConfig::MlGroupLocation);
    }

    /// The `"type"` discriminant tag must survive a JSON round-trip so the worker
    /// can always deserialize configs stored as JSONB in the database.
    #[test]
    fn job_config_type_tag_is_snake_case() {
        let cfg = JobConfig::GenThumbnail(GenThumbnailConfig {
            picture_id: Uuid::nil(),
            is_initial: true,
        });
        let json = serde_json::to_value(&cfg).unwrap();
        assert_eq!(
            json["type"].as_str().unwrap(),
            "gen_thumbnail",
            "type discriminant must be snake_case"
        );
    }
}

// ── Worker result types ───────────────────────────────────────────────────────

/// EXIF metadata extracted from a picture and returned in the job completion body.
/// The backend merges this into the `pictures` row.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractedExif {
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// The editable EXIF read from the file (promoted fields + camera/lens). The worker parses the
    /// raw EXIF capture timestamp into `exif.captured_at` at extraction time. Flattened, so the wire
    /// form is `{width, height, captured_at, gps_lat, …, camera_brand, …}`.
    #[serde(flatten)]
    pub exif: FullExif,
}
