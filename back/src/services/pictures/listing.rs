use crate::clients::federation::FederationClient;
use crate::domain::hierarchy::TagPredicate;
use crate::domain::picture::Picture;
use crate::domain::tag::TagPath;
use crate::infra::redis::{Cache, RedisKey};
use crate::infra::s3::{self, Storage};
use crate::infra::settings::keys;
use crate::repository::picture::{
    PictureListFilter, PictureRepository,
};
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::tag::TagRepository;
use crate::services::users::find_local_user_id;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;
use super::*;

#[tracing::instrument(skip(db), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn get_picture_details(
    db: &PgPool,
    user_id: Uuid,
    picture_id: Uuid,
) -> Result<PictureDetails, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    let versions = PictureVersionRepository::list_by_picture(db, picture_id).await?;
    Ok(PictureDetails { picture, versions })
}

/// Build the flat `TagPredicate` from the public list params (§6.3). Returns `None` when no flat
/// filter field is set. Comma-separated ltree paths; `match` selects AND/OR. No `exact`/
/// `minus_children` — hierarchy depth is only produced server-side by the resolver for `browse`.
fn build_flat_predicate(params: &PictureListParams) -> Result<Option<TagPredicate>, AppError> {
    fn split_parse(raw: &Option<String>) -> Result<Vec<TagPath>, AppError> {
        let Some(raw) = raw else { return Ok(vec![]) };
        raw.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            // Filtering (read-only) may reference protected `SharedToMe` paths.
            .map(|s| TagPath::parse(s, true).map_err(AppError::BadRequest))
            .collect()
    }

    let include = split_parse(&params.include_tags)?;
    let exclude = split_parse(&params.exclude_tags)?;
    // Single-valued since feature 35 §7 — multi-exact had no surface left.
    let exact = match params.exact.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => vec![TagPath::parse(s, true).map_err(AppError::BadRequest)?],
        None => vec![],
    };

    if !params.untagged && include.is_empty() && exclude.is_empty() && exact.is_empty() {
        return Ok(None);
    }

    let match_all = match params.match_mode.as_deref() {
        None | Some("all") => true,
        Some("any") => false,
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "invalid match mode {other:?} (expected \"all\" or \"any\")"
            )));
        }
    };

    Ok(Some(TagPredicate {
        include,
        match_all,
        exclude,
        untagged: params.untagged,
        exact,
        and_terms: vec![],
        minus_children: vec![],
    }))
}

#[tracing::instrument(skip(db, cache, storage, settings, federation, params), fields(user_id = %user_id))]
pub async fn list_pictures(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    user_id: Uuid,
    params: PictureListParams,
) -> Result<PictureListResult, AppError> {
    if params.page_size > 200 {
        return Err(AppError::BadRequest(
            "page_size cannot exceed 200".to_string(),
        ));
    }

    let predicate = build_flat_predicate(&params)?;

    let filter = PictureListFilter {
        page: params.page as i64,
        page_size: params.page_size as i64,
        sort: params.sort,
        order: params.order,
        predicate,
        owned_only: params.owned_only,
        shared_with_me: params.shared_with_me,
        trash: params.trash,
        captured_after: params.captured_after.map(|dt| dt.naive_utc()),
        captured_before: params.captured_before.map(|dt| dt.naive_utc()),
        gps: params.gps,
        capture_date: params.capture_date,
        missing_any: params.missing_any,
        near_time: params.near_time,
        near_lat: params.near_lat,
        near_lng: params.near_lng,
        undated_first: params.undated_first,
    };
    filter.validate()?;

    list_with_filter(
        db,
        cache,
        storage,
        settings,
        federation,
        user_id,
        filter,
        params.thumbnail,
    )
    .await
}

/// Run a picture list against a pre-built [`PictureListFilter`], presigning thumbnails for the
/// returned page. Shared by the public `GET /pictures` list and the hierarchy `browse` endpoint
/// (which builds its `filter.predicate` server-side from the resolver).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, filter), fields(user_id = %user_id))]
pub async fn list_with_filter(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    user_id: Uuid,
    filter: PictureListFilter,
    thumbnail: Option<ThumbnailSize>,
) -> Result<PictureListResult, AppError> {
    let page = filter.page as u32;
    let page_size = filter.page_size as u32;

    let (pictures, total) = PictureRepository::list(db, user_id, &filter).await?;

    // Owner identity for resolving owner-default creators on owned rows (feature 26 §5). Fetched
    // once — every row belongs to the caller, so the owner is always this user.
    let owner_username = crate::repository::user::UserRepository::find_by_id(db, user_id)
        .await?
        .map(|u| u.username)
        .unwrap_or_default();
    let global_domain = settings.get(keys::GLOBAL_DOMAIN);

    // Surface the per-row great-circle distance whenever a reference point is given (feature 29 §6),
    // regardless of the sort — so the photos-fix date mode can badge "distance from the picture being
    // fixed" without reordering the grid. Only geotagged rows get a value.
    let geo_ref = filter.near_lat.zip(filter.near_lng);

    // Batch-presign thumbnails: one cache lookup + one HTTP call per remote owner backend
    // instead of N sequential calls.
    let thumbnail_urls = if let Some(variant) = thumbnail {
        Some(
            presign_for_picture_list(
                db, cache, storage, &settings, federation, user_id, &pictures, variant,
            )
            .await?,
        )
    } else {
        None
    };

    let items = pictures
        .into_iter()
        .map(|pic| PictureListItem {
            id: pic.id,
            creator: pic.display_creator(&owner_username, &global_domain),
            filename: pic.filename,
            mime_type: pic.mime_type,
            width: pic.width,
            height: pic.height,
            captured_at: pic.captured_at,
            ingested_at: pic.ingested_at,
            updated_at: pic.updated_at,
            file_size: pic.file_size,
            original_file_created_at: pic.original_file_created_at,
            has_gps: pic.gps_lat.is_some() && pic.gps_lng.is_some(),
            distance_m: geo_ref
                .zip(pic.gps_lat.zip(pic.gps_lng))
                .map(|((ref_lat, ref_lng), (lat, lng))| haversine_m(ref_lat, ref_lng, lat, lng)),
            blurhash: pic.blurhash,
            orientation: pic.orientation,
            thumbnail_url: thumbnail_urls
                .as_ref()
                .and_then(|m| m.urls.get(&pic.id))
                .cloned(),
            owned: pic.remote_picture_id.is_none(),
            exif_sync_status: pic.exif_sync_status,
            deleted_at: pic.deleted_at,
            owner_deleted_at: pic.owner_deleted_at,
            owner_purge_at: pic.owner_purge_at,
            owner_reachable: thumbnail_urls
                .as_ref()
                .map(|m| !m.unreachable.contains(&pic.id))
                .unwrap_or(true),
            owner_username: pic.owner_username,
            owner_instance: pic.owner_instance_domain,
        })
        .collect();

    Ok(PictureListResult {
        total,
        page,
        page_size,
        items,
    })
}

/// Resolve presigned URLs for a list of pictures at the given variant in a single pass.
///
/// Strategy:
/// 1. Cache check for all pictures.
/// 2. Owned + same-backend cache misses: individual local S3 presigns (cheap, no network hop).
/// 3. Cross-instance cache misses: grouped by (owner_username, owner_instance) → one HTTP call
///    per remote owner backend instead of one call per picture.
#[tracing::instrument(skip(db, cache, storage, settings, federation, pictures, _local_user_id), fields(user_id = %_local_user_id))]
async fn presign_for_picture_list(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    _local_user_id: Uuid,
    pictures: &[Picture],
    variant: PictureVariant,
) -> Result<ListPresignResult, AppError> {
    let ttl = settings
        .get(keys::S3_PRESIGN_TTL_SECS)
        .saturating_sub(settings.get(keys::S3_PRESIGN_CACHE_MARGIN_SECS));

    let mut urls: HashMap<Uuid, String> = HashMap::new();
    let mut unreachable: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut misses: Vec<&Picture> = Vec::new();

    // Step 1: cache check. A thumbnail variant on a picture with no generated thumbnail (pending,
    // or a non-thumbnailable format like a PDF) gets no URL — left absent so the client renders a
    // file-type placeholder instead of a broken image.
    for pic in pictures {
        if variant.is_thumbnail() && pic.thumbnails_generated_at.is_none() {
            continue;
        }
        match cache
            .get_str(RedisKey::PictureUrl(pic.id, variant.as_str()))
            .await?
        {
            Some(url) => {
                urls.insert(pic.id, url);
            }
            None => misses.push(pic),
        }
    }

    if misses.is_empty() {
        return Ok(ListPresignResult { urls, unreachable });
    }

    // Step 2: classify cache misses
    let mut owned_misses: Vec<&Picture> = Vec::new();
    let mut same_backend_misses: Vec<(&Picture, Uuid)> = Vec::new();
    let mut cross_instance_groups: HashMap<(String, String), Vec<&Picture>> = HashMap::new();

    for pic in &misses {
        if pic.is_owned() {
            owned_misses.push(pic);
        } else {
            let owner_username = pic.owner_username.as_deref().unwrap_or_default();
            let owner_instance = pic.owner_instance_domain.as_deref().unwrap_or_default();
            if let Some(owner_id) =
                find_local_user_id(cache, db, settings, owner_username, owner_instance).await?
            {
                same_backend_misses.push((pic, owner_id));
            } else {
                cross_instance_groups
                    .entry((owner_username.to_string(), owner_instance.to_string()))
                    .or_default()
                    .push(pic);
            }
        }
    }

    // Step 3: presign owned pictures locally
    for pic in owned_misses {
        let key = s3::picture_key(pic.local_user_id, pic.id);
        let url = storage.presign_get(&variant.bucket(settings), &key).await?;
        if ttl > 0 {
            let _ = cache
                .set_str_ex(RedisKey::PictureUrl(pic.id, variant.as_str()), &url, ttl)
                .await;
        }
        urls.insert(pic.id, url);
    }

    // Step 4: presign same-backend received pictures locally (using sender's key)
    for (pic, owner_id) in same_backend_misses {
        let remote_id: Uuid = pic
            .remote_picture_id
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                AppError::InternalServerError("received picture missing remote_picture_id".into())
            })?;
        let key = s3::picture_key(owner_id, remote_id);
        let url = storage.presign_get(&variant.bucket(settings), &key).await?;
        if ttl > 0 {
            let _ = cache
                .set_str_ex(RedisKey::PictureUrl(pic.id, variant.as_str()), &url, ttl)
                .await;
        }
        urls.insert(pic.id, url);
    }

    // Step 5: batch-presign cross-instance pictures — one HTTP call per remote owner backend.
    // Each picture is authorised by its own per-picture token (stored on its incoming_share tag).
    for ((owner_username, owner_instance), pics) in &cross_instance_groups {
        // Resolve the per-picture token for each picture; skip any without an active token.
        let mut token_to_pic: HashMap<Uuid, &Picture> = HashMap::new();
        let mut batch: Vec<(Uuid, &str)> = Vec::new();
        for pic in pics {
            if let Some(token) = TagRepository::find_active_picture_token(db, pic.id).await? {
                token_to_pic.insert(token, pic);
                batch.push((token, variant.as_str()));
            }
        }
        if batch.is_empty() {
            continue;
        }

        // §3.1: isolate per owner-group. A down owner leaves *its* pictures' URLs absent and flags
        // them unreachable, but never fails the whole list — the caller's own pictures still render.
        let remote_urls = match federation
            .presign_remote_pictures(owner_username, owner_instance, &batch)
            .await
        {
            Ok(u) => u,
            Err(e) => {
                warn!(
                    owner_username,
                    owner_instance,
                    picture_count = token_to_pic.len(),
                    error = %e,
                    "federation: remote presign failed — marking owner unreachable"
                );
                unreachable.extend(token_to_pic.values().map(|p| p.id));
                continue;
            }
        };

        for (token, remote) in remote_urls {
            if let Some(pic) = token_to_pic.get(&token) {
                // §10: cache under a *truthful* lifetime — never past the owner's actual presign.
                let cache_ttl = truthful_cache_ttl(ttl, remote.expires_at);
                if cache_ttl > 0 {
                    let _ = cache
                        .set_str_ex(
                            RedisKey::PictureUrl(pic.id, variant.as_str()),
                            &remote.url,
                            cache_ttl,
                        )
                        .await;
                }
                urls.insert(pic.id, remote.url);
            }
        }
    }

    Ok(ListPresignResult { urls, unreachable })
}

/// Presigned URLs for a picture-list page, plus the ids of cross-instance pictures whose owner
/// backend was unreachable (feature 28 §3.2).
struct ListPresignResult {
    urls: HashMap<Uuid, String>,
    unreachable: std::collections::HashSet<Uuid>,
}

/// The cache TTL for a cross-instance presign: the local cap (`local_ttl`, already margin-adjusted),
/// bounded by the owner's advertised expiry so the cached URL is never advertised past the owner's
/// actual presign (feature 28 §10). A `None` remote expiry (peer predating the field) keeps the
/// local cap.
pub(super) fn truthful_cache_ttl(local_ttl: u64, remote_expires_at: Option<i64>) -> u64 {
    match remote_expires_at {
        Some(exp) => {
            let remaining = (exp - chrono::Utc::now().timestamp()).max(0) as u64;
            local_ttl.min(remaining)
        }
        None => local_ttl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> PictureListParams {
        PictureListParams {
            page: 1,
            page_size: 50,
            sort: Default::default(),
            order: Default::default(),
            include_tags: None,
            exclude_tags: None,
            exact: None,
            match_mode: None,
            untagged: false,
            owned_only: false,
            shared_with_me: false,
            trash: Default::default(),
            captured_after: None,
            captured_before: None,
            gps: Default::default(),
            capture_date: Default::default(),
            missing_any: false,
            near_time: None,
            near_lat: None,
            near_lng: None,
            undated_first: false,
            thumbnail: None,
        }
    }

    /// `exact` is the timeline's per-section scope, one path (feature 35 §7).
    #[test]
    fn exact_is_single_valued() {
        let p = PictureListParams {
            exact: Some("Era.2026.Vietnam".to_string()),
            ..params()
        };
        let pred = build_flat_predicate(&p).unwrap().unwrap();
        assert_eq!(pred.exact.len(), 1);
        assert_eq!(pred.exact[0].to_string(), "Era.2026.Vietnam");

        // A comma is no longer a separator, so a multi-value wire param is a bad path, not two tags.
        let multi = PictureListParams {
            exact: Some("Era.2026,Era.2025".to_string()),
            ..params()
        };
        assert!(matches!(
            build_flat_predicate(&multi),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn blank_exact_is_no_filter() {
        let p = PictureListParams {
            exact: Some("  ".to_string()),
            ..params()
        };
        assert!(build_flat_predicate(&p).unwrap().is_none());
    }

    /// The root's `direct` mode (34 §3.3) is `untagged`, and the gallery layers its cross-cutting
    /// `inc`/`exc` sets onto every query in the view — including that one (feature 35 §7).
    #[test]
    fn untagged_composes_with_tag_arms() {
        let p = PictureListParams {
            untagged: true,
            include_tags: Some("Era".to_string()),
            exclude_tags: Some("Screenshots".to_string()),
            ..params()
        };
        let pred = build_flat_predicate(&p).unwrap().unwrap();
        assert!(pred.untagged);
        assert_eq!(pred.include.len(), 1);
        assert_eq!(pred.exclude.len(), 1);
    }
}
