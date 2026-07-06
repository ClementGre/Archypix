//! Standalone-backend invite store (feature 23 §6.2). Redemption is atomic (check-validity +
//! increment in one statement). Shares the wire/domain type `archypix_common::registration::Invite`
//! (backend rows never carry an `instance_pin` — that's resolver-only).

use archypix_common::error::AppError;
use archypix_common::registration::Invite;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub struct InviteRepository;

impl InviteRepository {
    pub async fn create(
        db: &PgPool,
        code: &str,
        max_uses: Option<i64>,
        expires_at: Option<DateTime<Utc>>,
        created_by: &str,
    ) -> Result<Invite, AppError> {
        let row = sqlx::query!(
            r#"INSERT INTO invites (code, max_uses, expires_at, created_by)
               VALUES ($1, $2, $3, $4)
               RETURNING code, max_uses, uses, expires_at, created_by"#,
            code,
            max_uses,
            expires_at,
            created_by
        )
            .fetch_one(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(Invite {
            code: row.code,
            max_uses: row.max_uses,
            uses: row.uses,
            expires_at: row.expires_at,
            created_by: row.created_by,
            instance_pin: None,
        })
    }

    pub async fn list(db: &PgPool) -> Result<Vec<Invite>, AppError> {
        let rows = sqlx::query!(
            "SELECT code, max_uses, uses, expires_at, created_by FROM invites ORDER BY created_at DESC"
        )
            .fetch_all(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|r| Invite {
                code: r.code,
                max_uses: r.max_uses,
                uses: r.uses,
                expires_at: r.expires_at,
                created_by: r.created_by,
                instance_pin: None,
            })
            .collect())
    }

    /// Invites minted by a given user (the Profile-page / invite-manager list).
    pub async fn list_by(db: &PgPool, created_by: &str) -> Result<Vec<Invite>, AppError> {
        let rows = sqlx::query!(
            "SELECT code, max_uses, uses, expires_at, created_by FROM invites WHERE created_by = $1 ORDER BY created_at DESC",
            created_by
        )
            .fetch_all(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|r| Invite {
                code: r.code,
                max_uses: r.max_uses,
                uses: r.uses,
                expires_at: r.expires_at,
                created_by: r.created_by,
                instance_pin: None,
            })
            .collect())
    }

    pub async fn delete(db: &PgPool, code: &str) -> Result<(), AppError> {
        sqlx::query!("DELETE FROM invites WHERE code = $1", code)
            .execute(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(())
    }

    /// Look up an invite by code without redeeming it (public register-page preview).
    pub async fn find(db: &PgPool, code: &str) -> Result<Option<Invite>, AppError> {
        let row = sqlx::query!(
            "SELECT code, max_uses, uses, expires_at, created_by FROM invites WHERE code = $1",
            code
        )
            .fetch_optional(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(row.map(|r| Invite {
            code: r.code,
            max_uses: r.max_uses,
            uses: r.uses,
            expires_at: r.expires_at,
            created_by: r.created_by,
            instance_pin: None,
        }))
    }

    /// Atomically redeem `code`: increment `uses` iff the invite exists and is still valid, returning
    /// the redeemed invite (its `created_by` becomes the new user's `invited_by`). `None` means the
    /// code is unknown, expired, or exhausted. `max_uses = 0` is a **tracking invite** (open-mode
    /// referral): never exhausted, redeemed purely for provenance.
    pub async fn redeem(db: &PgPool, code: &str) -> Result<Option<Invite>, AppError> {
        let row = sqlx::query!(
            r#"UPDATE invites SET uses = uses + 1
               WHERE code = $1
                 AND (max_uses IS NULL OR max_uses = 0 OR uses < max_uses)
                 AND (expires_at IS NULL OR expires_at > now())
               RETURNING code, max_uses, uses, expires_at, created_by"#,
            code
        )
            .fetch_optional(db)
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        Ok(row.map(|r| Invite {
            code: r.code,
            max_uses: r.max_uses,
            uses: r.uses,
            expires_at: r.expires_at,
            created_by: r.created_by,
            instance_pin: None,
        }))
    }
}
