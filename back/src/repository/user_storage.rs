//! Storage-quota accounting (feature 22). The four billed counters are maintained by DB triggers
//! (`0007_storage_quotas`); this repository only reads them, seeds/clears the quota, and runs the
//! drift-correcting reconcile recompute.

use crate::infra::error::{AppError, map_sqlx_error};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// One user's billed usage breakdown (feature 22 §4.2). `billed_total` is the sum of the four cells.
#[derive(Debug, Clone, Default, serde::Serialize, sqlx::FromRow)]
pub struct UserStorage {
    pub originals_bytes: i64,
    pub originals_trashed_bytes: i64,
    pub versions_bytes: i64,
    pub versions_trashed_bytes: i64,
}

impl UserStorage {
    pub fn billed_total(&self) -> i64 {
        self.originals_bytes
            + self.originals_trashed_bytes
            + self.versions_bytes
            + self.versions_trashed_bytes
    }

    /// Trashed originals + trashed versions — the "empty trash to reclaim X" figure.
    pub fn reclaimable_trash_bytes(&self) -> i64 {
        self.originals_trashed_bytes + self.versions_trashed_bytes
    }
}

pub struct UserStorageRepository;

impl UserStorageRepository {
    /// The user's billed breakdown. Missing row (never touched a byte) reads as all-zero.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn get<'e, E>(ex: E, user_id: Uuid) -> Result<UserStorage, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as!(
            UserStorage,
            r#"SELECT originals_bytes, originals_trashed_bytes, versions_bytes, versions_trashed_bytes
               FROM user_storage WHERE user_id = $1"#,
            user_id,
        )
            .fetch_optional(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(row.unwrap_or_default())
    }

    /// The user's quota in bytes; `None` = unlimited.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn get_quota<'e, E>(ex: E, user_id: Uuid) -> Result<Option<i64>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            "SELECT storage_quota_bytes FROM users WHERE id = $1",
            user_id
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)
        .map(Option::flatten)
    }

    /// Set (or clear, with `None`) the user's quota. Returns the stored value.
    #[tracing::instrument(skip(ex), fields(user_id = %user_id))]
    pub async fn set_quota<'e, E>(
        ex: E,
        user_id: Uuid,
        quota_bytes: Option<i64>,
    ) -> Result<Option<i64>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_scalar!(
            r#"UPDATE users SET storage_quota_bytes = $2 WHERE id = $1
               RETURNING storage_quota_bytes"#,
            user_id,
            quota_bytes,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
        .map(|_| quota_bytes)
    }

    /// Recompute every user's four counters from scratch and overwrite `user_storage` — the
    /// drift-correcting reconcile (feature 22 §7). One set of grouped scans, not per-object work.
    /// Returns each user's refreshed billed total so the caller can refresh the Redis mirror.
    #[tracing::instrument(skip(ex))]
    pub async fn reconcile_all<'e, E>(ex: E) -> Result<Vec<(Uuid, i64)>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query!(
            r#"INSERT INTO user_storage (user_id, originals_bytes, originals_trashed_bytes,
                                         versions_bytes, versions_trashed_bytes)
               SELECT u.id,
                      COALESCE(o.live, 0), COALESCE(o.trash, 0),
                      COALESCE(v.live, 0), COALESCE(v.trash, 0)
               FROM users u
                        LEFT JOIN (SELECT local_user_id AS uid,
                                          SUM(file_size) FILTER (WHERE deleted_at IS NULL)     AS live,
                                          SUM(file_size) FILTER (WHERE deleted_at IS NOT NULL) AS trash
                                   FROM pictures WHERE remote_picture_id IS NULL
                                   GROUP BY local_user_id) o ON o.uid = u.id
                        LEFT JOIN (SELECT p.local_user_id AS uid,
                                          SUM(pv.file_size) FILTER (WHERE p.deleted_at IS NULL)     AS live,
                                          SUM(pv.file_size) FILTER (WHERE p.deleted_at IS NOT NULL) AS trash
                                   FROM picture_versions pv
                                            JOIN pictures p ON p.id = pv.picture_id
                                   WHERE p.remote_picture_id IS NULL
                                   GROUP BY p.local_user_id) v ON v.uid = u.id
               ON CONFLICT (user_id) DO UPDATE SET
                   originals_bytes         = EXCLUDED.originals_bytes,
                   originals_trashed_bytes = EXCLUDED.originals_trashed_bytes,
                   versions_bytes          = EXCLUDED.versions_bytes,
                   versions_trashed_bytes  = EXCLUDED.versions_trashed_bytes,
                   updated_at              = (now() AT TIME ZONE 'utc')
               RETURNING user_id,
                         (originals_bytes + originals_trashed_bytes
                          + versions_bytes + versions_trashed_bytes) AS "billed!""#,
        )
            .fetch_all(ex)
            .await
            .map_err(map_sqlx_error)?;
        Ok(rows.into_iter().map(|r| (r.user_id, r.billed)).collect())
    }
}
