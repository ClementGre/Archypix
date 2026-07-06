use crate::api::middleware::auth_user::AuthUser;
use crate::domain::tag::TagPath;
use crate::repository::tag::TagRepository;
use crate::services;
use crate::services::selection::{self, PictureSelection};
use crate::services::tags::TagBatchOutcome;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use std::collections::BTreeMap;
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

/// `PATCH /api/authenticated/tags` — add/remove tags across a [`PictureSelection`] (feature 14
/// §6.4). Accepts the selection descriptor or a legacy explicit `picture_ids` list. With
/// `dry_run: true` returns the §6.1 breakdown without mutating.
#[derive(Debug, Deserialize)]
pub struct EditPictureTagsRequest {
    #[serde(default)]
    pub selection: Option<PictureSelection>,
    /// Legacy explicit id list (used when `selection` is absent).
    #[serde(default)]
    pub picture_ids: Vec<Uuid>,
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default(), dry_run = payload.dry_run
))]
pub async fn edit(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<EditPictureTagsRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let add_tags = parse_tag_paths(&payload.add_tags)?;
    let remove_tags = parse_tag_paths(&payload.remove_tags)?;
    let sel = selection::resolve_or_explicit(
        &state.db,
        user_id,
        payload.selection.as_ref(),
        payload.picture_ids.clone(),
    )
    .await?;
    let outcome = services::tags::batch_edit_tags(
        &state.db,
        &state.routines.pipeline,
        user_id,
        &sel,
        &add_tags,
        &remove_tags,
        payload.dry_run,
    )
    .await?;
    Ok(Json(match outcome {
        TagBatchOutcome::DryRun(dry) => {
            serde_json::to_value(dry).map_err(|e| AppError::InternalServerError(e.to_string()))?
        }
        TagBatchOutcome::Applied { affected } => {
            serde_json::json!({ "ok": true, "affected": affected })
        }
    }))
}

/// `POST /api/authenticated/tags/rename` — rename a tag subtree everywhere the user references it
/// (edge case §7). Validates both paths, then triggers the async tag-rename cascade routine; the
/// caller gets an immediate ack. Both paths must be non-reserved ltree paths.
#[derive(Debug, Deserialize)]
pub struct RenameTagRequest {
    pub old_tag: String,
    pub new_tag: String,
}

#[tracing::instrument(skip(auth, state, payload), fields(user = %auth.claims.sub, user_id = %auth.claims.uid.unwrap_or_default()))]
pub async fn rename(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(payload): Json<RenameTagRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = auth.user_id()?;
    let old = TagPath::parse(&payload.old_tag, false).map_err(AppError::BadRequest)?;
    let new = TagPath::parse(&payload.new_tag, false).map_err(AppError::BadRequest)?;
    if old == new {
        return Err(AppError::BadRequest(
            "old_tag and new_tag are identical".to_string(),
        ));
    }
    if old.is_ancestor_of(&new) || new.is_ancestor_of(&old) {
        return Err(AppError::BadRequest(
            "cannot rename a tag into its own ancestor or descendant".to_string(),
        ));
    }
    state
        .routines
        .tag_rename
        .trigger(crate::infra::routine::tag_rename::TagRenameInput {
            user_id,
            old_tag: old.as_ltree().to_string(),
            new_tag: new.as_ltree().to_string(),
        });
    Ok(Json(serde_json::json!({ "ok": true })))
}
