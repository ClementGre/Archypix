//! Recipient-side EXIF model for received pictures (09_trash_and_exif_overrides §6, 10 §6.3).
//!
//! A received row keeps the owner's authoritative snapshot in `remote_exif_data` and the recipient's
//! sticky per-field overrides in `local_exif_overrides`. The materialised `exif_data` column (and the
//! promoted `captured_at`/`gps_*`/`orientation` columns the pipeline and rule predicates read) is the
//! merge `remote ‖ overrides` — an override key wins, every other field flows through from the owner.
//!
//! Overrides are stored as a raw canonical JSON object whose keys are the snake-case [`ExifField`]
//! names. Storing the explicit key **set** (not a diff against the owner) is what makes an override
//! sticky. A key **present with `null`** means the recipient claimed the field as **empty** (10 §6.3);
//! a key **absent** means un-claimed (the owner's value flows through). This third state is why the
//! override is a raw `Value`, not a sparse [`FullExif`] (whose `None` cannot distinguish the two).

use crate::domain::job::{CameraExif, ExifField, FullExif};
use chrono::NaiveDateTime;
use serde_json::{Map, Value};

/// The canonical JSON key for an [`ExifField`] (matches its serde snake_case rename).
pub fn field_key(f: ExifField) -> &'static str {
    match f {
        ExifField::CapturedAt => "captured_at",
        ExifField::GpsLat => "gps_lat",
        ExifField::GpsLng => "gps_lng",
        ExifField::GpsAlt => "gps_alt",
        ExifField::Orientation => "orientation",
        ExifField::CameraBrand => "camera_brand",
        ExifField::CameraModel => "camera_model",
        ExifField::FocalLengthMm => "focal_length_mm",
        ExifField::FNumber => "f_number",
        ExifField::IsoSpeed => "iso_speed",
        ExifField::ExposureTimeNum => "exposure_time_num",
        ExifField::ExposureTimeDen => "exposure_time_den",
    }
}

/// Lower a `set`/`empty`/`clear` override delta to the JSONB inputs the repository merge applies as
/// `(local_exif_overrides - clear_keys) || patch`: `patch` carries the `set` values plus an explicit
/// `null` for each `empty` field (the empty-claim, 10 §6.3); `clear_keys` are the dropped keys.
/// One builder for every recipient-override write — single, propose-escalate, and batch.
pub fn override_patch(
    set: &FullExif,
    empty: &[ExifField],
    clear: &[ExifField],
) -> (Value, Vec<String>) {
    // A `FullExif` flattens to a sparse object of its `Some` fields (canonical keys).
    let mut patch = match serde_json::to_value(set) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    for f in empty {
        patch.insert(field_key(*f).to_string(), Value::Null);
    }
    let clear_keys = clear.iter().map(|f| field_key(*f).to_string()).collect();
    (Value::Object(patch), clear_keys)
}

/// The materialised EXIF of a received row: the merged `exif_data` JSON plus the promoted columns,
/// recomputed whenever `remote_exif_data` or `local_exif_overrides` changes.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedExif {
    pub exif_data: Value,
    pub captured_at: Option<NaiveDateTime>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub orientation: Option<i16>,
}

impl MaterializedExif {
    /// The camera/lens part of the merge (what the `exif_data` column stores). Promoted keys are
    /// ignored on deserialisation; a `null` camera key materialises as `None` (claimed-empty).
    pub fn camera(&self) -> CameraExif {
        serde_json::from_value(self.exif_data.clone()).unwrap_or_default()
    }
}

/// Merge an owner snapshot with the recipient's sticky overrides (override key wins, including a
/// `null` claim → empty) and project out the promoted columns. The returned `exif_data` is the full
/// merged object.
pub fn materialize(remote: Option<&Value>, overrides: Option<&Value>) -> MaterializedExif {
    let mut merged = match remote {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    if let Some(Value::Object(ov)) = overrides {
        for (k, v) in ov {
            merged.insert(k.clone(), v.clone());
        }
    }
    let captured_at = merged
        .get("captured_at")
        .and_then(|v| serde_json::from_value::<NaiveDateTime>(v.clone()).ok());
    let gps_lat = merged.get("gps_lat").and_then(Value::as_f64);
    let gps_lng = merged.get("gps_lng").and_then(Value::as_f64);
    let gps_alt = merged
        .get("gps_alt")
        .and_then(Value::as_i64)
        .map(|n| n as i32);
    let orientation = merged
        .get("orientation")
        .and_then(Value::as_i64)
        .map(|n| n as i16);
    MaterializedExif {
        exif_data: Value::Object(merged),
        captured_at,
        gps_lat,
        gps_lng,
        gps_alt,
        orientation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").unwrap()
    }

    #[test]
    fn override_wins_per_field_and_owner_flows_through() {
        let remote = json!({ "captured_at": "2024-08-01T10:00:00", "gps_lat": 45.0, "gps_lng": 6.0, "orientation": 1, "camera_brand": "Canon" });
        // Recipient overrides only gps_lat.
        let overrides = json!({ "gps_lat": 48.0 });
        let m = materialize(Some(&remote), Some(&overrides));
        assert_eq!(m.gps_lat, Some(48.0), "override wins");
        assert_eq!(
            m.gps_lng,
            Some(6.0),
            "non-overridden owner value flows through"
        );
        assert_eq!(m.captured_at, Some(dt("2024-08-01T10:00:00")));
        assert_eq!(m.orientation, Some(1));
    }

    #[test]
    fn null_claim_empties_the_field_over_a_present_owner_value() {
        let remote = json!({ "gps_lat": 45.0, "gps_lng": 6.0, "camera_brand": "Canon" });
        // Recipient claims gps_lat as empty and camera_brand as empty.
        let overrides = json!({ "gps_lat": null, "camera_brand": null });
        let m = materialize(Some(&remote), Some(&overrides));
        assert_eq!(
            m.gps_lat, None,
            "null claim empties a promoted field over the owner value"
        );
        assert_eq!(
            m.gps_lng,
            Some(6.0),
            "a non-claimed field still flows through"
        );
        assert_eq!(
            m.camera().camera_brand,
            None,
            "null claim empties a camera field"
        );
    }

    #[test]
    fn override_patch_set_empty_clear() {
        // set gps_lat, empty orientation, clear gps_lng.
        let (patch, clear_keys) = override_patch(
            &FullExif {
                gps_lat: Some(1.5),
                ..Default::default()
            },
            &[ExifField::Orientation],
            &[ExifField::GpsLng],
        );
        let obj = patch.as_object().unwrap();
        assert_eq!(
            obj.get("gps_lat"),
            Some(&json!(1.5)),
            "set writes a value into the patch"
        );
        assert_eq!(
            obj.get("orientation"),
            Some(&Value::Null),
            "empty writes an explicit null claim"
        );
        assert!(
            obj.get("gps_lng").is_none(),
            "clear is a key drop, not a patch entry"
        );
        assert_eq!(
            clear_keys,
            vec!["gps_lng".to_string()],
            "clear becomes a removal key"
        );

        // The patch merges over existing overrides exactly as the SQL `(ov - clear) || patch` does.
        let existing = json!({ "gps_lng": 6.0, "iso_speed": 100 });
        let merged = materialize(Some(&json!({})), Some(&existing));
        assert_eq!(merged.gps_lng, Some(6.0));
        let _ = merged;
    }
}
