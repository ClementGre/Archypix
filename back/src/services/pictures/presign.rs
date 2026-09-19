use crate::clients::federation::FederationClient;
use crate::domain::picture::Picture;
use crate::infra::redis::{Cache, RedisKey};
use crate::infra::s3::{self, Storage};
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::tag::TagRepository;
use crate::services::users::find_local_user_id;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use tracing::trace;
use uuid::Uuid;
use super::*;
use super::listing::truthful_cache_ttl;

#[tracing::instrument(skip(db, cache, storage, settings, federation), fields(user_id = %local_user_id, picture_id = %picture_id))]
pub async fn presign_picture_variant(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    local_user_id: Uuid,
    picture_id: Uuid,
    variant: PictureVariant,
) -> Result<Option<String>, AppError> {
    let pic = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if pic.local_user_id != local_user_id {
        return Err(AppError::NotFound);
    }

    presign_variant_for_picture(db, cache, storage, settings, federation, &pic, variant).await
}

/// Presign one already-authorized picture at `variant` (owned → local S3 key; same-backend received →
/// the sender's key; cross-instance received → the picture's token + a remote presign). The single
/// owned/received branch shared by the authenticated `presign_picture_variant` (ownership-gated) and
/// the public-share presign (coverage-gated) — the caller does the authorization, this does the S3
/// work + cache. Returns `None` for a thumbnail variant that doesn't exist yet.
#[tracing::instrument(skip(db, cache, storage, settings, federation, pic), fields(picture_id = %pic.id))]
pub async fn presign_variant_for_picture(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    pic: &Picture,
    variant: PictureVariant,
) -> Result<Option<String>, AppError> {
    // No thumbnail to presign (pending, or a non-thumbnailable format) → `None` so the client shows
    // a file-type placeholder. The `original` always exists.
    if variant.is_thumbnail() && pic.thumbnails_generated_at.is_none() {
        return Ok(None);
    }

    // Single cache check for all picture types (owned, same-backend share, cross-instance share).
    if let Some(cached) = cache
        .get_str(RedisKey::PictureUrl(pic.id, variant.as_str()))
        .await?
    {
        trace!("presign cache hit");
        return Ok(Some(cached));
    }

    // `(url, remote_expires_at)` — the remote expiry (if any) bounds the cache lifetime (§10).
    let (url, remote_expires_at): (String, Option<i64>) = if pic.is_owned() {
        let key = s3::picture_key(pic.local_user_id, pic.id);
        (
            storage
                .presign_get(&variant.bucket(&settings), &key)
                .await?,
            None,
        )
    } else {
        let owner_username = pic.owner_username.as_deref().unwrap_or_default();
        let owner_instance = pic.owner_instance_domain.as_deref().unwrap_or_default();
        // The remote picture's UUID on the owner's backend is stored as remote_picture_id.
        let remote_id: Uuid = pic
            .remote_picture_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AppError::InternalServerError("received picture missing remote_picture_id".into())
            })?;

        // Check if the owner lives on this backend (resolver setup allows multiple backends per
        // global domain). Cache the lookup to avoid a DB hit on every picture in a listing.
        if let Some(owner_id) =
            find_local_user_id(cache, db, settings, owner_username, owner_instance).await?
        {
            // Owner is on this backend — derive S3 key from their user_id + original picture id.
            let key = s3::picture_key(owner_id, remote_id);
            (
                storage.presign_get(&variant.bucket(settings), &key).await?,
                None,
            )
        } else {
            // Owner is on a different backend — authorise via the picture's own token and call
            // remote. A transient owner-unreachable failure surfaces as `503` (§3.3), distinct from
            // `Ok(None)` (no thumbnail exists), so the frontend shows a retryable error.
            let picture_token = TagRepository::find_active_picture_token(db, pic.id)
                .await?
                .ok_or_else(|| {
                    AppError::Unauthorized(format!(
                        "No active presign token for picture {}",
                        pic.id
                    ))
                })?;
            let mut urls = federation
                .presign_remote_pictures(
                    owner_username,
                    owner_instance,
                    &[(picture_token, variant.as_str())],
                )
                .await?;
            let remote = urls.remove(&picture_token).ok_or_else(|| {
                AppError::InternalServerError(format!(
                    "Remote backend did not return presigned URL for picture {}",
                    pic.id
                ))
            })?;
            (remote.url, remote.expires_at)
        }
    };

    let ttl = settings
        .get(keys::S3_PRESIGN_TTL_SECS)
        .saturating_sub(settings.get(keys::S3_PRESIGN_CACHE_MARGIN_SECS));
    let cache_ttl = truthful_cache_ttl(ttl, remote_expires_at);
    if cache_ttl > 0 {
        cache
            .set_str_ex(
                RedisKey::PictureUrl(pic.id, variant.as_str()),
                &url,
                cache_ttl,
            )
            .await?;
    }
    Ok(Some(url))
}

