use crate::domain::picture::PictureVersion;
use archypix_common::error::{map_sqlx_error, AppError};
use sqlx::{Executor, Postgres};
use uuid::Uuid;

pub struct PictureVersionRepository;

impl PictureVersionRepository {
    /// `id` must be the same UUID used to key the S3 object
    /// (`{user_id}/{picture_id}/{id}`), so it can always be reconstructed from the DB.
    #[tracing::instrument(skip(ex), fields(version_id = %id, picture_id = %picture_id))]
    pub async fn create<'e, E>(
        ex: E,
        id: Uuid,
        picture_id: Uuid,
        version_number: i32,
        file_size: Option<i64>,
        mime_type: Option<&str>,
    ) -> Result<PictureVersion, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PictureVersion,
            r#"INSERT INTO picture_versions (id, picture_id, version_number, file_size, mime_type)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, picture_id, version_number, file_size, mime_type, created_at"#,
            id,
            picture_id,
            version_number,
            file_size,
            mime_type,
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)
    }

    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn list_by_picture<'e, E>(
        ex: E,
        picture_id: Uuid,
    ) -> Result<Vec<PictureVersion>, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as!(
            PictureVersion,
            r#"SELECT id, picture_id, version_number, file_size, mime_type, created_at
               FROM picture_versions
               WHERE picture_id = $1
               ORDER BY version_number ASC"#,
            picture_id,
        )
        .fetch_all(ex)
        .await
        .map_err(map_sqlx_error)
    }

    /// Whether the picture already has at least one stored version. Drives the versioning
    /// snapshot predicate (§9): `OriginalCopy` keeps only the first snapshot.
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn has_versions<'e, E>(ex: E, picture_id: Uuid) -> Result<bool, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exists: Option<bool> = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM picture_versions WHERE picture_id = $1)",
            picture_id
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)?;
        Ok(exists.unwrap_or(false))
    }

    /// Returns MAX(version_number) + 1 for the given picture, defaulting to 1 if no versions exist.
    #[tracing::instrument(skip(ex), fields(picture_id = %picture_id))]
    pub async fn next_version_number<'e, E>(ex: E, picture_id: Uuid) -> Result<i32, AppError>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let max: Option<i32> = sqlx::query_scalar!(
            "SELECT MAX(version_number) FROM picture_versions WHERE picture_id = $1",
            picture_id
        )
        .fetch_one(ex)
        .await
        .map_err(map_sqlx_error)?;

        Ok(max.unwrap_or(0) + 1)
    }
}
