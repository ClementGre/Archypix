//! Registration modes + invite domain logic (feature 23 §6), shared by the resolver (multi-instance
//! enforcement) and a standalone backend (local enforcement). Pure logic + wire types; each crate
//! owns its `invites` table and does the *atomic* redemption (check-validity-and-increment) in SQL.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How new-user registration is gated. The variant list lives here only; the wire strings
/// (`open`/`invite`/`admin_invite`) and its `SettingType` impl are derived (see [`crate::wire_enum`]).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::VariantNames,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RegistrationMode {
    /// Anyone registers, no invite. Invites are still *mintable* (for instance-pinning).
    Open,
    /// A valid invite is required; **any existing user** may mint invites.
    Invite,
    /// A valid invite is required; **only admins** may mint invites.
    AdminInvite,
}

// Make `RegistrationMode` usable as a first-class setting value (`SettingKey<RegistrationMode>`).
#[cfg(feature = "settings")]
crate::wire_enum!(RegistrationMode);

impl RegistrationMode {
    pub fn requires_invite(&self) -> bool {
        matches!(
            self,
            RegistrationMode::Invite | RegistrationMode::AdminInvite
        )
    }

    /// Whether a user (admin or not) may mint an invite under this mode.
    pub fn can_mint(&self, is_admin: bool) -> bool {
        match self {
            RegistrationMode::Open | RegistrationMode::Invite => true,
            RegistrationMode::AdminInvite => is_admin,
        }
    }

    pub fn as_str(&self) -> &'static str {
        (*self).into()
    }
}

impl Default for RegistrationMode {
    fn default() -> Self {
        RegistrationMode::Open
    }
}

/// Number of base36 characters in a generated invite code — short enough to type/read (`abc-def-ghi`).
pub const INVITE_CODE_LEN: usize = 9;

/// Generate a short random invite code: 9 lowercase base36 chars (`[a-z0-9]`), displayed grouped
/// (`ABC-DEF-GHI`) but stored/urls lowercase.
#[cfg(feature = "registration")]
pub fn generate_invite_code() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..INVITE_CODE_LEN)
        .map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// A shared invite record. `instance_pin` is resolver-only (a `back_domain` suggestion); standalone
/// backends leave it `None`.
///
/// **`max_uses` semantics:** `Some(n>0)` = capped at `n`; `Some(0)` = unlimited (uncapped invitation);
/// `None` = a **tracking referral link** — unlimited but only valid in `open` registration mode
/// (a pure provenance link). When the mode is not `open`, tracking invites are inactive (tombstoned).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    /// `Some(0)` = unlimited, `None` = tracking referral (open-only), `Some(n)` = capped.
    pub max_uses: Option<i64>,
    pub uses: i64,
    /// `None` = never expires.
    pub expires_at: Option<DateTime<Utc>>,
    /// Username of the minter (the future `users.invited_by`).
    pub created_by: String,
    /// Resolver-only pinned backend suggestion.
    pub instance_pin: Option<String>,
}

impl Invite {
    /// A **tracking referral link** (`max_uses = None`): unlimited, valid only in `open` mode, purely
    /// for provenance. (An uncapped *invitation* is `Some(0)`.)
    pub fn is_tracking(&self) -> bool {
        self.max_uses.is_none()
    }

    /// Remaining uses (`None` = unlimited — both tracking and `Some(0)`).
    pub fn remaining(&self) -> Option<i64> {
        self.max_uses.filter(|&m| m > 0).map(|m| (m - self.uses).max(0))
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|e| e <= now)
    }

    pub fn is_exhausted(&self) -> bool {
        // `None` (tracking) and `Some(0)` (uncapped) never exhaust; only a positive cap does.
        self.max_uses.is_some_and(|m| m > 0 && self.uses >= m)
    }

    /// Whether the invite may still be redeemed at `now` in `mode`. A tracking (`None`) invite is
    /// only redeemable in `open` mode; capped/uncapped invitations require a gated mode is fine too.
    pub fn is_active(&self, mode: RegistrationMode, now: DateTime<Utc>) -> bool {
        if self.is_expired(now) || self.is_exhausted() {
            return false;
        }
        // A tracking link is a no-op outside open mode (registration is gated there).
        !(self.is_tracking() && mode != RegistrationMode::Open)
    }

    /// Mode-agnostic redeemability (expiry + exhaustion only). Prefer [`Invite::is_active`] when the
    /// registration mode is known.
    pub fn is_valid(&self, now: DateTime<Utc>) -> bool {
        !self.is_expired(now) && !self.is_exhausted()
    }
}

/// Why a registration attempt was rejected on invite grounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    InviteRequired,
    InviteNotFound,
    InviteExpired,
    InviteExhausted,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RegistrationError::InviteRequired => "an invite code is required to register",
            RegistrationError::InviteNotFound => "the invite code is invalid",
            RegistrationError::InviteExpired => "the invite code has expired",
            RegistrationError::InviteExhausted => "the invite code has no remaining uses",
        };
        f.write_str(s)
    }
}

impl std::error::Error for RegistrationError {}

/// Decide whether a signup may proceed given the mode and the looked-up invite (if any).
///
/// - `Open`: always allowed; a supplied invite is honoured only if still valid (used for pinning),
///   an invalid one is ignored (returns `Ok(None)`).
/// - `Invite`/`AdminInvite`: a present, valid invite is mandatory.
///
/// Returns the invite that should be *redeemed* (its `created_by` becomes `invited_by`), or `None`.
/// This is the pre-check; the caller still performs the atomic increment in SQL.
pub fn authorize_registration<'a>(
    mode: RegistrationMode,
    invite: Option<&'a Invite>,
    code_supplied: bool,
    now: DateTime<Utc>,
) -> Result<Option<&'a Invite>, RegistrationError> {
    if !mode.requires_invite() {
        // Open: honour a valid pinned/tracking invite, ignore an invalid/absent one.
        return Ok(invite.filter(|i| i.is_active(mode, now)));
    }
    if !code_supplied {
        return Err(RegistrationError::InviteRequired);
    }
    let invite = invite.ok_or(RegistrationError::InviteNotFound)?;
    if invite.is_expired(now) {
        return Err(RegistrationError::InviteExpired);
    }
    if invite.is_exhausted() {
        return Err(RegistrationError::InviteExhausted);
    }
    // A tracking referral link is meaningless in a gated mode — treat it as not a real invite.
    if invite.is_tracking() {
        return Err(RegistrationError::InviteNotFound);
    }
    Ok(Some(invite))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(max: Option<i64>, uses: i64, exp: Option<DateTime<Utc>>) -> Invite {
        Invite {
            code: "c".into(),
            max_uses: max,
            uses,
            expires_at: exp,
            created_by: "alice".into(),
            instance_pin: None,
        }
    }

    #[test]
    fn open_ignores_absent_invite() {
        let r = authorize_registration(RegistrationMode::Open, None, false, Utc::now()).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn open_honours_valid_pin() {
        let inv = invite(Some(5), 0, None);
        let r =
            authorize_registration(RegistrationMode::Open, Some(&inv), true, Utc::now()).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn uncapped_and_tracking_never_exhaust() {
        // Some(0) = uncapped invitation (works in any mode); None = tracking referral (open-only).
        let uncapped = invite(Some(0), 1000, None);
        assert!(!uncapped.is_tracking());
        assert!(!uncapped.is_exhausted());
        assert_eq!(uncapped.remaining(), None);
        assert!(uncapped.is_active(RegistrationMode::Invite, Utc::now()));

        let tracking = invite(None, 1000, None);
        assert!(tracking.is_tracking());
        assert!(!tracking.is_exhausted());
        assert!(tracking.is_active(RegistrationMode::Open, Utc::now()));
        // A tracking link is inactive (tombstoned) once the mode is gated.
        assert!(!tracking.is_active(RegistrationMode::Invite, Utc::now()));
    }

    #[test]
    fn tracking_honoured_in_open_rejected_when_gated() {
        let tracking = invite(None, 5, None);
        let r =
            authorize_registration(RegistrationMode::Open, Some(&tracking), true, Utc::now()).unwrap();
        assert!(r.is_some(), "tracking referral is honoured for provenance in open mode");
        let e = authorize_registration(RegistrationMode::Invite, Some(&tracking), true, Utc::now())
            .unwrap_err();
        assert_eq!(e, RegistrationError::InviteNotFound);
    }

    #[test]
    fn open_ignores_exhausted_pin() {
        let inv = invite(Some(1), 1, None);
        let r =
            authorize_registration(RegistrationMode::Open, Some(&inv), true, Utc::now()).unwrap();
        assert!(
            r.is_none(),
            "an exhausted pin is ignored, not an error, in Open mode"
        );
    }

    #[test]
    fn invite_mode_requires_code() {
        let e =
            authorize_registration(RegistrationMode::Invite, None, false, Utc::now()).unwrap_err();
        assert_eq!(e, RegistrationError::InviteRequired);
    }

    #[test]
    fn invite_mode_rejects_missing_record() {
        let e =
            authorize_registration(RegistrationMode::Invite, None, true, Utc::now()).unwrap_err();
        assert_eq!(e, RegistrationError::InviteNotFound);
    }

    #[test]
    fn invite_mode_rejects_exhausted_and_expired() {
        let now = Utc::now();
        let exhausted = invite(Some(2), 2, None);
        assert_eq!(
            authorize_registration(RegistrationMode::Invite, Some(&exhausted), true, now)
                .unwrap_err(),
            RegistrationError::InviteExhausted
        );
        let expired = invite(None, 0, Some(now - chrono::Duration::seconds(1)));
        assert_eq!(
            authorize_registration(RegistrationMode::Invite, Some(&expired), true, now)
                .unwrap_err(),
            RegistrationError::InviteExpired
        );
    }

    #[test]
    fn can_mint_rules() {
        assert!(RegistrationMode::Open.can_mint(false));
        assert!(RegistrationMode::Invite.can_mint(false));
        assert!(!RegistrationMode::AdminInvite.can_mint(false));
        assert!(RegistrationMode::AdminInvite.can_mint(true));
    }
}
