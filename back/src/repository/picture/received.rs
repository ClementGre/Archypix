use crate::domain::job::CameraExif;
use archypix_common::error::{AppError, map_sqlx_error};
use chrono::NaiveDateTime;
use sqlx::{Executor, Postgres};
use uuid::Uuid;
use super::*;

impl PictureRepository {
    /// The last-applied owner `updated_at` for a received row, for the stale-announcement guard
    /// (feature 28 §7). Outer `None` ⇒ no such received row yet; inner `None` ⇒ a row from a peer
    /// that predated the field. Keyed by the received-picture unique `(local_user_id, remote_picture_id)`.
    pub async fn received_remote_updated_at<'e, E>(
        ex: E,
        recipient_id: Uuid,
        remote_picture_id: &str,
    ) -> Result<Option<Option<NaiveDateTime>>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query!(
            r#"SELECT remote_updated_at FROM pictures
               WHERE local_user_id = $1 AND remote_picture_id = $2"#,
            recipient_id,
            remote_picture_id,
        )
        .fetch_optional(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.map(|r| r.remote_updated_at))
    }

    /// Re-materialise a received row's `exif_data` + promoted columns from the
    /// `merge(remote_exif_data, local_exif_overrides)` the caller computed (09 §6/§8). Bumps
    /// `updated_at` (announcement re-delivery gate) and re-dirties the row for the local `metadata`
    /// event (date/GPS rules re-evaluate on the merged EXIF).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(ex, camera), fields(picture_id = %id))]
    pub async fn apply_received_materialization<'e, E>(
        ex: E,
        id: Uuid,
        camera: &CameraExif,
        captured_at: Option<NaiveDateTime>,
        gps_lat: Option<f64>,
        gps_lng: Option<f64>,
        gps_alt: Option<i32>,
        orientation: Option<i16>,
    ) -> Result<(), AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exif_data = serde_json::to_value(camera)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        sqlx::query!(
            r#"UPDATE pictures
               SET exif_data   = $2,
                   captured_at = $3,
                   gps_lat     = $4,
                   gps_lng     = $5,
                   gps_alt     = $6,
                   orientation = $7,
                   last_pipeline_run_at = NULL
               WHERE id = $1"#,
            id,
            exif_data,
            captured_at,
            gps_lat,
            gps_lng,
            gps_alt,
            orientation,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(())
    }

    /// Delete received-picture rows from `sender` for `recipient_id` that have no remaining
    /// `incoming_share` tags.
    ///
    /// Called after `TagRepository::remove_incoming_share_tags` during share revocation.
    ///
    /// A revoked picture is unreachable regardless of any local tags Bob may have added —
    /// the sender's presign endpoint will reject requests once the share token is invalid.
    /// Manual tags are therefore not a reason to keep the row.
    ///
    /// Pictures received from the same sender via a *different, still-active* share survive:
    /// they retain `incoming_share` tags from that other share, so the `NOT EXISTS` check
    /// excludes them.
    ///
    /// Returns the number of deleted rows.
    #[tracing::instrument(skip(ex), fields(user_id = %recipient_id))]
    pub async fn delete_received_without_share_tags<'e, E>(
        ex: E,
        recipient_id: Uuid,
        sender_username: &str,
        sender_instance: &str,
    ) -> Result<u64, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query!(
            r#"DELETE FROM pictures
               WHERE local_user_id = $1
                 AND owner_username = $2
                 AND owner_instance_domain = $3
                 AND remote_picture_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM tags
                     WHERE tags.picture_id = pictures.id
                       AND tags.source = 'incoming_share'::tag_source
                 )"#,
            recipient_id,
            sender_username,
            sender_instance,
        )
        .execute(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(result.rows_affected())
    }

    /// From `candidate_picture_ids`, return those that still carry at least one
    /// `incoming_share` source tag (i.e. survived a share's tag cleanup). Used by
    /// `cleanup_incoming_share` to mark survivors dirty for token refresh.
    #[tracing::instrument(skip(ex, candidate_picture_ids), fields(user_id = %recipient_id))]
    pub async fn find_with_any_incoming_share_tag<'e, E>(
        ex: E,
        recipient_id: Uuid,
        candidate_picture_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if candidate_picture_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"SELECT DISTINCT p.id
               FROM pictures p
               JOIN tags t ON t.picture_id = p.id
               WHERE p.id = ANY($1::uuid[])
                 AND p.local_user_id = $2
                 AND t.source = 'incoming_share'::tag_source"#,
            candidate_picture_ids as &[Uuid],
            recipient_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Map a set of `remote_picture_id` strings to the recipient's local picture ids.
    /// Used by per-picture unannounce to resolve the sender's announce ids locally.
    #[tracing::instrument(skip(ex, remote_ids), fields(user_id = %recipient_id))]
    pub async fn find_ids_by_remote_ids<'e, E>(
        ex: E,
        recipient_id: Uuid,
        remote_ids: &[String],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if remote_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"SELECT id FROM pictures
               WHERE local_user_id = $1
                 AND remote_picture_id = ANY($2::text[])"#,
            recipient_id,
            remote_ids as &[String],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Narrow `ids` to the ones the user actually holds (trashed included — a trashed picture is
    /// still a legitimate tag cover, feature 34 §3.2).
    #[tracing::instrument(skip(ex, ids), fields(user_id = %local_user_id))]
    pub async fn filter_owned_ids<'e, E>(
        ex: E,
        local_user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            "SELECT id FROM pictures WHERE local_user_id = $1 AND id = ANY($2::uuid[])",
            local_user_id,
            ids as &[Uuid],
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Delete the received pictures in `picture_ids` that have no remaining `incoming_share`
    /// tag. Returns the deleted ids. Used by per-picture unannounce.
    #[tracing::instrument(skip(ex, picture_ids), fields(user_id = %recipient_id))]
    pub async fn delete_orphans_among<'e, E>(
        ex: E,
        recipient_id: Uuid,
        picture_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if picture_ids.is_empty() {
            return Ok(vec![]);
        }
        sqlx::query_scalar!(
            r#"DELETE FROM pictures
               WHERE id = ANY($1::uuid[])
                 AND local_user_id = $2
                 AND remote_picture_id IS NOT NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM tags
                     WHERE tags.picture_id = pictures.id
                       AND tags.source = 'incoming_share'::tag_source
                 )
               RETURNING id"#,
            picture_ids as &[Uuid],
            recipient_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

}
