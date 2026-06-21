use crate::api::middleware::auth_user::AuthUser;
use crate::domain::tag::TagPath;
use crate::infra::error::AppError;
use crate::repository::pipeline::PipelineRepository;
use crate::repository::tag::TagRepository;
use crate::services;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use std::collections::BTreeMap;
use tracing::debug;
use uuid::Uuid;

fn parse_tag_paths(paths: &[String]) -> Result<Vec<String>, AppError> {
    paths
        .iter()
        .map(|p| {
            TagPath::parse(p, false)
                .map(|t| t.as_ltree().to_string())
                .map_err(AppError::BadRequest)
        })
        .collect()
}

#[derive(Debug, Deserialize)]
pub struct ListTagsQuery {
    pub picture_id: Option<Uuid>,
    /// When true (and `picture_id` is set), return each tag with the list of sources that
    /// assert it, instead of the folded display set.
    #[serde(default)]
    pub with_sources: bool,
}

#[tracing::instrument(skip(auth, state, query), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ListTagsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;

    if let Some(picture_id) = query.picture_id {
        let tags = TagRepository::list_for_picture(&state.db, user_id, picture_id).await?;

        if query.with_sources {
            // Group per path, preserving the per-source provenance. Sorted for stable output.
            let mut by_path: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
            for tag in tags {
                by_path
                    .entry(tag.tag_path)
                    .or_default()
                    .push(serde_json::json!({ "source": tag.source, "source_id": tag.source_id }));
            }
            let items: Vec<serde_json::Value> = by_path
                .into_iter()
                .map(|(path, sources)| serde_json::json!({ "path": path, "sources": sources }))
                .collect();
            return Ok(Json(serde_json::json!({ "tags": items })));
        }

        // Default view: fold per-source rows to the deepest distinct paths.
        let folded =
            TagPath::fold_deepest(tags.into_iter().map(|t| TagPath::from_ltree(t.tag_path)));
        let paths: Vec<String> = folded
            .into_iter()
            .map(|p| p.as_ltree().to_string())
            .collect();
        return Ok(Json(serde_json::json!({ "tags": paths })));
    }

    let tags = TagRepository::list_paths_by_user(&state.db, user_id).await?;
    Ok(Json(serde_json::json!({ "tags": tags })))
}

#[derive(Debug, Deserialize)]
pub struct EditPictureTagsRequest {
    pub picture_ids: Vec<Uuid>,
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn edit(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<EditPictureTagsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let add_tags = parse_tag_paths(&payload.add_tags)?;
    let remove_tags = parse_tag_paths(&payload.remove_tags)?;
    services::tags::edit_picture_tags(
        &state.db,
        auth.user_id()?,
        &payload.picture_ids,
        &add_tags,
        &remove_tags,
    )
    .await?;
    // Manual tag changes invalidate these pictures so the pipeline re-evaluates
    // requires/excludes conditions that may now be satisfied or violated.
    if let Err(e) = PipelineRepository::invalidate(&state.db, &payload.picture_ids).await {
        tracing::error!(error = ?e, "failed to invalidate pipeline for edited pictures");
    }
    state.pipeline_waker.wake(auth.user_id()?);
    Ok(Json(serde_json::json!({ "ok": true })))
}
