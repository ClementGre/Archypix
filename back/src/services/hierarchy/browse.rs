use crate::clients::federation::FederationClient;
use crate::infra::redis::Cache;
use crate::infra::s3::Storage;
use crate::repository::picture::{
    PictureListFilter, PictureSortField, PresenceFilter, SortOrder, TrashFilter,
};
use crate::services::pictures::{PictureListResult, ThumbnailSize};
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use super::*;
use super::crud::{load_owned, parse_config};


#[derive(Debug, Clone)]
pub struct BrowseParams {
    pub page: u32,
    pub page_size: u32,
    pub sort: PictureSortField,
    pub order: SortOrder,
    pub trash: TrashFilter,
    pub owned_only: bool,
    pub shared_with_me: bool,
    pub captured_after: Option<DateTime<Utc>>,
    pub captured_before: Option<DateTime<Utc>>,
    /// Presence + proximity params (feature 29 §4, §6) threaded through the directory listing.
    pub gps: PresenceFilter,
    pub capture_date: PresenceFilter,
    pub missing_any: bool,
    pub near_time: Option<NaiveDateTime>,
    pub near_lat: Option<f64>,
    pub near_lng: Option<f64>,
    pub thumbnail: Option<ThumbnailSize>,
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, params), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
pub async fn browse(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    user_id: Uuid,
    hierarchy_id: Uuid,
    path: &str,
    params: BrowseParams,
) -> Result<PictureListResult, AppError> {
    if params.page_size > 200 {
        return Err(AppError::BadRequest(
            "page_size cannot exceed 200".to_string(),
        ));
    }
    let row = load_owned(db, user_id, hierarchy_id).await?;
    let hierarchy_config = parse_config(&row.config)?;
    let root = resolve_for_user(db, user_id, &hierarchy_config).await?;

    let segments = split_path(path);
    let target = find_dir(&root, &segments).ok_or(AppError::NotFound)?;

    // static directories (and any None-direct node) have no direct files.
    let Some(predicate) = target.direct.clone() else {
        return Ok(PictureListResult {
            total: 0,
            page: params.page,
            page_size: params.page_size,
            items: vec![],
        });
    };

    let filter = PictureListFilter {
        page: params.page as i64,
        page_size: params.page_size as i64,
        sort: params.sort,
        order: params.order,
        predicate: Some(predicate),
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
        // Hierarchy browse order is directory/name-based, so date-fix float-to-top does not apply
        // (feature 30 §12 edge 10).
        undated_first: false,
    };
    filter.validate()?;

    crate::services::pictures::list_with_filter(
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

