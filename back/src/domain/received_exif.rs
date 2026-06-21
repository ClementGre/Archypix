//! Recipient-side EXIF model for received pictures (09_trash_and_exif_overrides §6).
//!
//! A received row keeps the owner's authoritative snapshot in `remote_exif_data` and the recipient's
//! sticky per-field overrides in `local_exif_overrides`. The materialised `exif_data` column (and the
//! promoted `captured_at`/`gps_*`/`orientation` columns the pipeline and rule predicates read) is the
//! merge `remote ‖ overrides` — an override key wins, every other field flows through from the owner.
//!
//! Both `remote_exif_data` and `local_exif_overrides` use one **canonical full editable-EXIF JSON**
//! object whose keys are the snake-case [`ExifField`] names: the five *promoted* keys
//! (`captured_at`, `gps_lat`, `gps_lng`, `gps_alt`, `orientation`) plus the camera/lens keys
//! (`camera_brand`, …). Storing the explicit key **set** (not a diff against the owner) is what makes
//! an override sticky: an owner later setting a field to the recipient's value does not silently
//! transfer ownership of that field back to the owner.

use crate::domain::job::{ExifField, ExifOverrides};
use chrono::NaiveDateTime;
use serde_json::{Map, Value};

/// The five editable-EXIF keys promoted to their own `pictures` columns.
pub const PROMOTED_KEYS: [&str; 5] = [
    "captured_at",
    "gps_lat",
    "gps_lng",
    "gps_alt",
    "orientation",
];

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

/// Insert the `Some` fields of an [`ExifOverrides`] into `target` (canonical keys). `None` fields are
/// left untouched — clearing a field is the caller's job (remove the key).
pub fn apply_overrides_into(target: &mut Map<String, Value>, set: &ExifOverrides) {
    macro_rules! put {
        ($field:ident, $key:literal) => {
            if let Some(v) = &set.$field {
                target.insert(
                    $key.to_string(),
                    serde_json::to_value(v).unwrap_or(Value::Null),
                );
            }
        };
    }
    put!(captured_at, "captured_at");
    put!(gps_lat, "gps_lat");
    put!(gps_lng, "gps_lng");
    put!(gps_alt, "gps_alt");
    put!(orientation, "orientation");
    put!(camera_brand, "camera_brand");
    put!(camera_model, "camera_model");
    put!(focal_length_mm, "focal_length_mm");
    put!(f_number, "f_number");
    put!(iso_speed, "iso_speed");
    put!(exposure_time_num, "exposure_time_num");
    put!(exposure_time_den, "exposure_time_den");
}

/// Build the owner's canonical snapshot JSON for a received row from the announced components:
/// the camera/lens `exif_data` object plus the promoted typed fields.
#[allow(clippy::too_many_arguments)]
pub fn build_owner_exif(
    captured_at: Option<NaiveDateTime>,
    gps_lat: Option<f64>,
    gps_lng: Option<f64>,
    gps_alt: Option<i32>,
    orientation: Option<i16>,
    camera_exif: Option<&Value>,
) -> Value {
    let mut obj = match camera_exif {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    // The promoted typed fields override any same-named key that slipped into the camera object.
    if let Some(v) = captured_at {
        obj.insert(
            "captured_at".into(),
            serde_json::to_value(v).unwrap_or(Value::Null),
        );
    }
    if let Some(v) = gps_lat {
        obj.insert("gps_lat".into(), Value::from(v));
    }
    if let Some(v) = gps_lng {
        obj.insert("gps_lng".into(), Value::from(v));
    }
    if let Some(v) = gps_alt {
        obj.insert("gps_alt".into(), Value::from(v));
    }
    if let Some(v) = orientation {
        obj.insert("orientation".into(), Value::from(v));
    }
    Value::Object(obj)
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

/// Merge an owner snapshot with the recipient's sticky overrides (override key wins) and project
/// out the promoted columns. The returned `exif_data` is the full merged object.
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

/// The owner snapshot decomposed back into announce components: the camera/lens `exif_data`
/// (promoted keys stripped) plus the promoted typed fields. Used when a relayer forwards a received
/// picture downstream — it announces the **owner** snapshot it holds, never its merged `exif_data`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OwnerSnapshot {
    pub camera_exif: Value,
    pub captured_at: Option<NaiveDateTime>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub orientation: Option<i16>,
}

/// Split a stored `remote_exif_data` snapshot into [`OwnerSnapshot`] announce components.
pub fn decompose(remote: Option<&Value>) -> OwnerSnapshot {
    let mut obj = match remote {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let captured_at = obj
        .remove("captured_at")
        .and_then(|v| serde_json::from_value::<NaiveDateTime>(v).ok());
    let gps_lat = obj.remove("gps_lat").as_ref().and_then(Value::as_f64);
    let gps_lng = obj.remove("gps_lng").as_ref().and_then(Value::as_f64);
    let gps_alt = obj
        .remove("gps_alt")
        .as_ref()
        .and_then(Value::as_i64)
        .map(|n| n as i32);
    let orientation = obj
        .remove("orientation")
        .as_ref()
        .and_then(Value::as_i64)
        .map(|n| n as i16);
    OwnerSnapshot {
        camera_exif: Value::Object(obj),
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
    fn build_and_decompose_round_trip() {
        let camera = json!({ "camera_brand": "Canon", "iso_speed": 100 });
        let remote = build_owner_exif(
            Some(dt("2024-08-01T10:00:00")),
            Some(45.8),
            Some(6.8),
            Some(1200),
            Some(6),
            Some(&camera),
        );
        let snap = decompose(Some(&remote));
        assert_eq!(snap.captured_at, Some(dt("2024-08-01T10:00:00")));
        assert_eq!(snap.gps_lat, Some(45.8));
        assert_eq!(snap.gps_alt, Some(1200));
        assert_eq!(snap.orientation, Some(6));
        assert_eq!(
            snap.camera_exif,
            json!({ "camera_brand": "Canon", "iso_speed": 100 })
        );
    }

    #[test]
    fn override_wins_per_field_and_owner_flows_through() {
        let remote = build_owner_exif(
            Some(dt("2024-08-01T10:00:00")),
            Some(45.0),
            Some(6.0),
            None,
            Some(1),
            Some(&json!({ "camera_brand": "Canon" })),
        );
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

        // Owner re-announces with a new captured_at and a new gps_lat; the override stays sticky.
        let remote2 = build_owner_exif(
            Some(dt("2024-09-09T09:00:00")),
            Some(10.0),
            Some(6.0),
            None,
            Some(1),
            Some(&json!({ "camera_brand": "Canon" })),
        );
        let m2 = materialize(Some(&remote2), Some(&overrides));
        assert_eq!(
            m2.gps_lat,
            Some(48.0),
            "override remains sticky after owner edit"
        );
        assert_eq!(
            m2.captured_at,
            Some(dt("2024-09-09T09:00:00")),
            "owner edit to a non-overridden field flows through"
        );
    }

    #[test]
    fn apply_overrides_builds_canonical_keys() {
        let mut obj = Map::new();
        apply_overrides_into(
            &mut obj,
            &ExifOverrides {
                gps_lat: Some(1.5),
                orientation: Some(8),
                ..Default::default()
            },
        );
        assert_eq!(obj.get("gps_lat"), Some(&json!(1.5)));
        assert_eq!(obj.get("orientation"), Some(&json!(8)));
        assert!(obj.get("gps_lng").is_none());
    }
}
