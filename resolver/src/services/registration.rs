//! Registration-mode enforcement + invite redemption for the resolver's `/api/public/register`
//! (feature 23 §6). In resolver mode the resolver is authoritative; the backend accepts every
//! forwarded signup. Shares the mode/invite domain logic with the standalone backend via
//! `common::registration`.

use crate::config::{Config, setting_keys as sk};
use crate::repository;
use archypix_common::error::AppError;
use archypix_common::registration::{Invite, RegistrationError, authorize_registration};
use chrono::Utc;
use sqlx::PgPool;

/// The outcome of authorising a signup: the redeemed invite (its `instance_pin` steers placement and
/// its `created_by` becomes `invited_by`), if any.
pub struct Authorized {
    pub invite: Option<Invite>,
}

impl Authorized {
    pub fn instance_pin(&self) -> Option<&str> {
        self.invite.as_ref().and_then(|i| i.instance_pin.as_deref())
    }
    pub fn invited_by(&self) -> Option<String> {
        self.invite.as_ref().map(|i| i.created_by.clone())
    }
}

fn map_err(e: RegistrationError) -> AppError {
    AppError::BadRequest(e.to_string())
}

/// Enforce the current mode + atomically redeem an invite if one is required/supplied.
pub async fn authorize(
    db: &PgPool,
    config: &Config,
    code: Option<&str>,
) -> Result<Authorized, AppError> {
    let mode = config.get(sk::REGISTRATION_MODE);
    let code = code.map(str::trim).filter(|c| !c.is_empty());

    // Pre-check against the current (non-atomic) view, then redeem atomically.
    let looked_up = match code {
        Some(c) => repository::get_invite(db, c).await?,
        None => None,
    };
    authorize_registration(mode, looked_up.as_ref(), code.is_some(), Utc::now())
        .map_err(map_err)?;

    // If an invite must (or may, for pinning) be consumed, redeem it atomically now.
    let invite = if code.is_some() {
        match repository::redeem_invite(db, code.unwrap()).await? {
            Some(inv) => Some(inv),
            // Lost a race since the pre-check (exhausted/expired between). Required modes fail.
            None if mode.requires_invite() => {
                return Err(AppError::BadRequest(
                    "the invite code is no longer valid".to_string(),
                ));
            }
            None => None,
        }
    } else {
        None
    };

    Ok(Authorized { invite })
}
