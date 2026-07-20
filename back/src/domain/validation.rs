//! Small input validators shared across services (07_security_audit.md §2.4/§2.7).

use crate::domain::job::{ExifField, FullExif};

/// Any GPS component present pulls in all three (they are set/cleared/emptied as a unit).
pub fn couple_gps(mut fields: Vec<ExifField>) -> Vec<ExifField> {
    if fields
        .iter()
        .any(|f| matches!(f, ExifField::GpsLat | ExifField::GpsLng | ExifField::GpsAlt))
    {
        for f in [ExifField::GpsLat, ExifField::GpsLng, ExifField::GpsAlt] {
            if !fields.contains(&f) {
                fields.push(f);
            }
        }
    }
    fields
}

/// Normalise an EXIF edit delta shared by every edit path (owned write-through, recipient-local
/// override, batch): GPS-couple `empty`/`clear`, reject a field claimed in both `set` and
/// `empty`/`clear`, and range-check `set`. Returns the normalised `(empty, clear)`.
pub fn validate_exif_edit(
    set: &FullExif,
    empty: Vec<ExifField>,
    clear: Vec<ExifField>,
) -> Result<(Vec<ExifField>, Vec<ExifField>), String> {
    let empty = couple_gps(empty);
    let clear = couple_gps(clear);

    for &f in empty.iter().chain(clear.iter()) {
        if set.has(f) {
            return Err(format!("field {f:?} present in both set and empty/clear"));
        }
    }
    if let Some(lat) = set.gps_lat {
        if !(-90.0..=90.0).contains(&lat) {
            return Err("gps_lat out of range [-90,90]".into());
        }
    }
    if let Some(lng) = set.gps_lng {
        if !(-180.0..=180.0).contains(&lng) {
            return Err("gps_lng out of range [-180,180]".into());
        }
    }
    if let Some(o) = set.orientation {
        if !(1..=8).contains(&o) {
            return Err("orientation must be 1..=8".into());
        }
    }
    Ok((empty, clear))
}

/// Minimum acceptable password length.
pub const MIN_PASSWORD_LEN: usize = 8;

/// Validate a password meets the minimum policy. Returns the human-readable reason on failure.
pub fn validate_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!(
            "Password must be at least {MIN_PASSWORD_LEN} characters long"
        ));
    }
    Ok(())
}

/// Loose syntactic email check: a single `@` with a non-empty local part and a dotted domain.
/// Not RFC-5322 exhaustive — just enough to reject obviously malformed input. Uniqueness and
/// deliverability are out of scope (uniqueness is enforced by the DB constraint).
pub fn validate_email(email: &str) -> Result<(), String> {
    let email = email.trim();
    let mut parts = email.split('@');
    let (local, domain) = match (parts.next(), parts.next(), parts.next()) {
        (Some(l), Some(d), None) => (l, d),
        _ => return Err("Email must contain exactly one '@'".to_string()),
    };
    if local.is_empty() {
        return Err("Email local part must not be empty".to_string());
    }
    if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
        return Err("Email domain is invalid".to_string());
    }
    if email.chars().any(|c| c.is_whitespace()) {
        return Err("Email must not contain whitespace".to_string());
    }
    Ok(())
}

/// Validate a user-supplied federation **global domain** (e.g. a share `recipient_instance`).
///
/// This is the user-controlled input that drives an outbound resolution + federation HTTP call, so
/// it is the primary blind-SSRF / request-amplification surface (07_security_audit.md §2.4). We
/// require a plain dotted hostname and reject schemes, paths, ports, whitespace, `localhost`, and
/// IP-address literals (which would point at internal/metadata endpoints). Legitimate federation
/// always targets a real global domain; internal backend URLs come from the trusted resolver, not
/// from this field.
pub fn validate_federation_domain(domain: &str) -> Result<(), String> {
    let d = domain.trim();
    if d.is_empty() {
        return Err("recipient_instance must not be empty".to_string());
    }
    if d.contains("://") || d.contains('/') || d.contains(':') || d.contains('@') {
        return Err(
            "recipient_instance must be a bare domain (no scheme, port, or path)".to_string(),
        );
    }
    if d.chars().any(|c| c.is_whitespace()) {
        return Err("recipient_instance must not contain whitespace".to_string());
    }
    if !d.contains('.') {
        return Err("recipient_instance must be a fully-qualified domain".to_string());
    }
    let lower = d.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
        return Err("recipient_instance must not be a local domain".to_string());
    }
    // Reject IPv4 / IPv6 literals — federation identities are domains, and an IP literal here is a
    // classic SSRF pivot (e.g. 169.254.169.254, 127.0.0.1, 10.x, ::1).
    if d.parse::<std::net::IpAddr>().is_ok() || d.starts_with('[') {
        return Err("recipient_instance must be a domain, not an IP address".to_string());
    }
    // Each label must look like a hostname label.
    for label in d.split('.') {
        if label.is_empty()
            || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            || label.starts_with('-')
            || label.ends_with('-')
        {
            return Err(format!("recipient_instance has an invalid label {label:?}"));
        }
    }
    Ok(())
}

/// Maximum length of a share name (matches the `outgoing_shares.name` / `incoming_shares.name`
/// VARCHAR(64) column).
pub const MAX_SHARE_NAME_LEN: usize = 64;
/// Maximum length of a share message.
pub const MAX_SHARE_MESSAGE_LEN: usize = 1000;

/// Validate a share `name`: required, non-blank, and within the column length. Returns the
/// human-readable reason on failure.
pub fn validate_share_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Share name must not be empty".to_string());
    }
    if name.chars().count() > MAX_SHARE_NAME_LEN {
        return Err(format!(
            "Share name must be at most {MAX_SHARE_NAME_LEN} characters long"
        ));
    }
    Ok(())
}

/// Validate an optional share `message`: only the length is constrained (an empty message is fine —
/// callers should normalise it to `None`).
pub fn validate_share_message(message: Option<&str>) -> Result<(), String> {
    if let Some(message) = message {
        if message.chars().count() > MAX_SHARE_MESSAGE_LEN {
            return Err(format!(
                "Share message must be at most {MAX_SHARE_MESSAGE_LEN} characters long"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords() {
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password("short").is_err());
    }

    #[test]
    fn emails() {
        assert!(validate_email("me@example.com").is_ok());
        assert!(validate_email("a.b@sub.example.co.uk").is_ok());
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("a@b").is_err());
        assert!(validate_email("a@@b.com").is_err());
        assert!(validate_email("a b@example.com").is_err());
    }

    #[test]
    fn share_names() {
        assert!(validate_share_name("Alps trip").is_ok());
        assert!(validate_share_name("  ").is_err());
        assert!(validate_share_name("").is_err());
        assert!(validate_share_name(&"a".repeat(MAX_SHARE_NAME_LEN)).is_ok());
        assert!(validate_share_name(&"a".repeat(MAX_SHARE_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn share_messages() {
        assert!(validate_share_message(None).is_ok());
        assert!(validate_share_message(Some("Here are the photos")).is_ok());
        assert!(validate_share_message(Some(&"a".repeat(MAX_SHARE_MESSAGE_LEN))).is_ok());
        assert!(validate_share_message(Some(&"a".repeat(MAX_SHARE_MESSAGE_LEN + 1))).is_err());
    }

    #[test]
    fn exif_edit() {
        use crate::domain::job::ExifField;
        // GPS-couples empty/clear; a lone GPS component pulls in all three.
        let (empty, clear) =
            validate_exif_edit(&FullExif::default(), vec![ExifField::GpsLat], vec![]).unwrap();
        assert_eq!(
            empty.len(),
            3,
            "empty GPS is coupled to all three components"
        );
        assert!(clear.is_empty());
        // Range checks.
        let bad = FullExif {
            gps_lat: Some(120.0),
            ..Default::default()
        };
        assert!(validate_exif_edit(&bad, vec![], vec![]).is_err());
        let bad = FullExif {
            orientation: Some(9),
            ..Default::default()
        };
        assert!(validate_exif_edit(&bad, vec![], vec![]).is_err());
        // A field cannot be both set and emptied/cleared.
        let set = FullExif {
            orientation: Some(3),
            ..Default::default()
        };
        assert!(validate_exif_edit(&set, vec![ExifField::Orientation], vec![]).is_err());
    }

    #[test]
    fn federation_domains() {
        assert!(validate_federation_domain("other.example.com").is_ok());
        assert!(validate_federation_domain("instance.co").is_ok());
        assert!(validate_federation_domain("localhost").is_err());
        assert!(validate_federation_domain("127.0.0.1").is_err());
        assert!(validate_federation_domain("169.254.169.254").is_err());
        assert!(validate_federation_domain("::1").is_err());
        assert!(validate_federation_domain("https://evil.com").is_err());
        assert!(validate_federation_domain("evil.com:8080").is_err());
        assert!(validate_federation_domain("evil.com/path").is_err());
        assert!(validate_federation_domain("nodot").is_err());
        assert!(validate_federation_domain("box.local").is_err());
    }
}
