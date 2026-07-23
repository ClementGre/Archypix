//! Public shares (feature 27): the link-gated *pull* share service.
//!
//! Three seams, three mechanics (spec §1):
//! - **View** — token → live tag-coverage query → presign, all on the owner backend. No pipeline,
//!   no tracking table, no per-picture tokens. Revocation is instant.
//! - **Contribute** — an anonymous upload lands as a picture *owned by the share owner*, tagged into
//!   the album, `creator = #name`. Runs through the existing upload-time dedup (a hash hit against the
//!   owner's live/trashed pictures is rejected, never stored).
//! - **Convert** — an authenticated visitor Subscribes (a recipient-initiated derived `OutgoingShare`
//!   via the `shares/public/claim` verb) and/or saves a copy (feature 11). Only Convert re-enters the
//!   share pipeline.

use crate::clients::federation::FederationClient;
use crate::clients::federation::models::PublicShareClaimResponse;
use crate::domain::auth::TokenType;
use crate::domain::hierarchy::TagPredicate;
use crate::domain::public_share::{PublicPermissions, PublicShare, contribution_creator};
use crate::domain::share::ShareStatus;
use crate::domain::tag::TagPath;
use crate::infra::crypto::{self, JwtService};
use crate::infra::ratelimit;
use crate::infra::redis::Cache;
use crate::infra::routine::RoutineHandle;
use crate::infra::routine::unannounce::UnannounceInput;
use crate::infra::s3::Storage;
use crate::infra::settings::keys;
use crate::repository::picture::{
    PictureListFilter, PictureRepository, PictureSortField, ResolvedSelection, SortOrder,
};
use crate::repository::public_share::PublicShareRepository;
use crate::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use crate::repository::user::UserRepository;
use crate::services::aggregate::{AggregateRequest, AggregateSection, aggregate};
use crate::services::federation::receive_public_claim;
use crate::services::pictures::{
    self, BatchUploadFile, BatchUploadOutcome, PictureListResult, PictureVariant, UploadMetadata,
};
use crate::services::selection::PictureSelection;
use crate::services::shares::revoke_outgoing_share;
use crate::services::users::find_local_user_id;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use chrono::{NaiveDateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// The coverage `PictureListFilter`: live (non-deleted, non-hidden) pictures tagged `<@ tag_path`.
/// Owner scoping is applied by the repository (`local_user_id = owner`).
fn coverage_filter(
    tag_ltree: &str,
    page: i64,
    page_size: i64,
    sort: PictureSortField,
    order: SortOrder,
) -> PictureListFilter {
    PictureListFilter {
        page,
        page_size,
        sort,
        order,
        predicate: Some(TagPredicate {
            include: vec![TagPath::from_ltree(tag_ltree.to_string())],
            match_all: true,
            ..TagPredicate::all()
        }),
        ..Default::default()
    }
}

/// The image/video MIME allowlist for anonymous contributions (§13) — the existing ingestable set.
fn is_ingestable_public_mime(mime: &str) -> bool {
    archypix_common::mime::supports_thumbnail(mime) || archypix_common::mime::supports_exif(mime)
}

// ── View (unauthenticated, token-gated) ─────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PublicShareMeta {
    pub name: String,
    pub message: Option<String>,
    pub owner_display: String,
    pub tag_path: String,
    pub permissions: PublicPermissions,
    pub picture_count: i64,
    pub requires_password: bool,
    pub expires_at: Option<NaiveDateTime>,
    pub view_only: bool,
}

/// Public metadata for the share landing page. Returned even for a locked share (so the frontend
/// knows to show the password gate) — but never the pictures.
#[tracing::instrument(skip(db, settings))]
pub async fn public_meta(
    db: &PgPool,
    settings: &Settings,
    token: &str,
) -> Result<PublicShareMeta, AppError> {
    let share = accessible_share(db, token).await?;
    let owner = UserRepository::find_by_id(db, share.owner_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let owner_display = crate::domain::picture::Picture::format_identity(
        &owner.username,
        &settings.get(keys::GLOBAL_DOMAIN),
    );
    let filter = coverage_filter(
        &share.tag_path,
        1,
        1,
        PictureSortField::default(),
        SortOrder::default(),
    );
    let picture_count = PictureRepository::count(db, share.owner_id, &filter).await?;
    let permissions = share.permissions();
    let requires_password = share.requires_password();
    let view_only = share.view_only();
    let expires_at = share.expires_at;
    Ok(PublicShareMeta {
        tag_path: share.tag_path.clone(),
        name: share.name,
        message: share.message,
        owner_display,
        permissions,
        picture_count,
        requires_password,
        expires_at,
        view_only,
    })
}

/// Find an accessible (active + unexpired) share by token, or `404`. Hides revoked/expired/unknown
/// behind one status so the token is no oracle.
async fn accessible_share(db: &PgPool, token: &str) -> Result<PublicShare, AppError> {
    PublicShareRepository::find_by_token(db, token)
        .await?
        .filter(|s| s.is_accessible(Utc::now().naive_utc()))
        .ok_or(AppError::NotFound)
}

/// Resolve a share for a request, enforcing the optional password gate. For a password-gated share the
/// caller must present a valid unlock JWT (`bearer`) minted for exactly this share (§6).
#[tracing::instrument(skip(db, jwt, settings, bearer))]
pub async fn resolve_access(
    db: &PgPool,
    jwt: &JwtService,
    settings: &Settings,
    token: &str,
    bearer: Option<&str>,
) -> Result<PublicShare, AppError> {
    let share = accessible_share(db, token).await?;
    if share.requires_password() {
        let bearer = bearer
            .ok_or_else(|| AppError::Unauthorized("this share requires a password".to_string()))?;
        let claims = jwt.decode(bearer, &settings.get(keys::BACK_DOMAIN))?;
        if claims.token_type != TokenType::PublicShare || claims.sub != share.id.to_string() {
            return Err(AppError::Unauthorized(
                "invalid or expired unlock session".to_string(),
            ));
        }
    }
    Ok(share)
}

/// Verify the password and mint a short-lived unlock JWT (`TokenType::PublicShare`, `sub = share.id`).
#[tracing::instrument(skip(db, jwt, settings, password))]
pub async fn unlock(
    db: &PgPool,
    jwt: &JwtService,
    settings: &Settings,
    token: &str,
    password: &str,
) -> Result<String, AppError> {
    let share = accessible_share(db, token).await?;
    let hash = share
        .password_hash
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("this share is not password-protected".to_string()))?;
    if !crypto::verify_password(password, hash)? {
        return Err(AppError::Unauthorized("incorrect password".to_string()));
    }
    let jwt_token = jwt.issue(
        &share.id.to_string(),
        None,
        &settings.get(keys::GLOBAL_DOMAIN),
        TokenType::PublicShare,
        false,
        &settings.get(keys::BACK_DOMAIN),
        settings.get(keys::PUBLIC_SHARE_SESSION_TTL_SECS),
    )?;
    Ok(jwt_token)
}

/// Paginated coverage listing with presigned thumbnails (reuses `list_with_filter` with the owner as
/// the user + the coverage predicate). A view-only share strips `captured_at` from the payload
/// (thumbnails carry no EXIF; the presigned variant is a thumbnail, never the original).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, share))]
pub async fn list_public_pictures(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    share: &PublicShare,
    page: u32,
    page_size: u32,
    thumbnail: PictureVariant,
) -> Result<PictureListResult, AppError> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 200);
    let filter = coverage_filter(
        &share.tag_path,
        page as i64,
        page_size as i64,
        PictureSortField::CapturedAt,
        SortOrder::Desc,
    );
    let mut result = pictures::list_with_filter(
        db,
        cache,
        storage,
        settings,
        federation,
        share.owner_id,
        filter,
        Some(thumbnail),
    )
    .await?;
    if share.view_only() {
        for item in &mut result.items {
            item.captured_at = None;
        }
    }
    Ok(result)
}

/// Coverage-checked presign (§6): the picture must be in the share's live coverage. A view-only share
/// presigns thumbnail variants only. Received pictures in coverage proxy to the real owner (the
/// owned/received branching in `presign_variant_for_picture`).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, share))]
pub async fn presign_public_picture(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    share: &PublicShare,
    picture_id: Uuid,
    variant: PictureVariant,
) -> Result<Option<String>, AppError> {
    if share.view_only() && !variant.is_thumbnail() {
        return Err(AppError::Forbidden(
            "this share does not allow downloading original files".to_string(),
        ));
    }
    let pic = PublicShareRepository::find_covered_picture(
        db,
        share.owner_id,
        &share.tag_path,
        picture_id,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    pictures::presign_variant_for_picture(db, cache, storage, settings, federation, &pic, variant)
        .await
}

/// A single covered picture's detail for the public info panel. A view-only share omits
/// EXIF/GPS/`captured_at` (§6). `creator` is the resolved display credit (owner default, origin, or
/// `#contributor`).
#[derive(Debug, Serialize)]
pub struct PublicPictureDetail {
    pub id: Uuid,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub blurhash: Option<String>,
    pub orientation: Option<i16>,
    pub ingested_at: NaiveDateTime,
    pub creator: String,
    pub captured_at: Option<NaiveDateTime>,
    pub gps_lat: Option<f64>,
    pub gps_lng: Option<f64>,
    pub gps_alt: Option<i32>,
    pub exif_data: Option<Value>,
}

/// Fetch a covered picture's detail (coverage-checked), resolving the creator against the album owner
/// and stripping EXIF/GPS for a view-only share.
#[tracing::instrument(skip(db, settings, share))]
pub async fn public_picture_detail(
    db: &PgPool,
    settings: &Settings,
    share: &PublicShare,
    picture_id: Uuid,
) -> Result<PublicPictureDetail, AppError> {
    let p = PublicShareRepository::find_covered_picture(
        db,
        share.owner_id,
        &share.tag_path,
        picture_id,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    // Owner identity resolves an owned owner-default (`creator IS NULL`) to `@owner:domain`.
    let owner_username = UserRepository::find_by_id(db, share.owner_id)
        .await?
        .map(|u| u.username)
        .unwrap_or_default();
    let creator = p.propagated_creator(&owner_username, &settings.get(keys::GLOBAL_DOMAIN));
    let view_only = share.view_only();
    Ok(PublicPictureDetail {
        id: p.id,
        filename: p.filename.clone(),
        mime_type: p.mime_type.clone(),
        file_size: p.file_size,
        width: p.width,
        height: p.height,
        blurhash: p.blurhash.clone(),
        orientation: p.orientation,
        ingested_at: p.ingested_at,
        creator,
        captured_at: (!view_only).then_some(p.captured_at).flatten(),
        gps_lat: (!view_only).then_some(p.gps_lat).flatten(),
        gps_lng: (!view_only).then_some(p.gps_lng).flatten(),
        gps_alt: (!view_only).then_some(p.gps_alt).flatten(),
        exif_data: (!view_only).then(|| serde_json::to_value(&p.exif_data).unwrap_or(Value::Null)),
    })
}

/// Batch EXIF/summary aggregation over a **coverage-intersected** explicit selection (§6). Only the
/// visitor's explicitly-selected picture ids are honoured (no union-of-ids escape), and they are
/// filtered to the share's coverage first, so a visitor can never aggregate outside the album. A
/// view-only share drops the EXIF section.
#[tracing::instrument(skip(db, settings, share, include_ids, sections))]
pub async fn public_aggregate(
    db: &PgPool,
    settings: &Settings,
    share: &PublicShare,
    include_ids: Vec<Uuid>,
    sections: Option<Vec<AggregateSection>>,
) -> Result<Value, AppError> {
    let covered = PublicShareRepository::filter_covered_ids(
        db,
        share.owner_id,
        &share.tag_path,
        &include_ids,
    )
    .await?;
    let mut sections = sections.unwrap_or_else(|| vec![AggregateSection::Summary]);

    // Retain unauthorized sections.
    sections.retain(|s| *s != AggregateSection::Tags);
    if share.view_only() {
        sections.retain(|s| *s != AggregateSection::Exif);
    }
    let request = AggregateRequest {
        selection: PictureSelection {
            query: None,
            include_ids: covered,
            exclude_ids: vec![],
        },
        sections: Some(sections),
        tag_provenance: false,
    };
    // The album owner is the aggregate principal; their identity resolves owner-default creators.
    let owner_username = UserRepository::find_by_id(db, share.owner_id)
        .await?
        .map(|u| u.username)
        .unwrap_or_default();
    aggregate(db, settings, share.owner_id, &owner_username, request).await
}

// ── Contribution (anonymous upload, §7) ─────────────────────────────────────────

/// Presign an anonymous contribution batch: gate `allow_upload`, enforce per-request caps + the per-IP
/// / per-share rate limit, then run the existing upload-time dedup (charged to the owner's quota). A
/// hash hit against the owner's live/trashed pictures is **rejected** (a `Duplicate` outcome, not
/// stored) — the boomerang-style protection the spec requires, and never auto-tagged into the album.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, waker, share, files))]
pub async fn public_upload_batch(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    waker: &RoutineHandle<Uuid>,
    share: &PublicShare,
    client_ip: &str,
    files: &[BatchUploadFile],
) -> Result<Vec<BatchUploadOutcome>, AppError> {
    if !share.allow_upload {
        return Err(AppError::Forbidden(
            "this album does not accept contributions".to_string(),
        ));
    }
    let max_files = settings.get(keys::PUBLIC_UPLOAD_MAX_FILES_PER_REQUEST);
    if files.len() > max_files {
        return Err(AppError::BadRequest(format!(
            "at most {max_files} files per upload request"
        )));
    }
    let max_bytes = settings.get(keys::PUBLIC_UPLOAD_MAX_FILE_BYTES);
    if files.iter().any(|f| f.size.is_some_and(|s| s > max_bytes)) {
        return Err(AppError::PayloadTooLarge(
            "a contributed file exceeds the size limit".to_string(),
        ));
    }
    ratelimit::check_categorized(
        cache,
        ratelimit::category::PUBLIC_UPLOAD,
        &format!("pubup:{}:{}", share.id, client_ip),
        settings.get(keys::PUBLIC_UPLOAD_RATE_MAX),
        settings.get(keys::PUBLIC_UPLOAD_RATE_WINDOW_SECS),
        settings.get(keys::RATE_LIMIT_EVENT_RETENTION_SECS),
    )
    .await?;
    // No `initial_tags`/`upload_label`: a dedup hit must NOT be auto-tagged into the album (that would
    // surface the owner's private/trashed content) — it is simply rejected (§14).
    pictures::begin_upload_batch(
        db,
        cache,
        storage,
        settings,
        share.owner_id,
        files,
        &[],
        None,
        waker,
    )
    .await
}

/// Complete an anonymous contribution: validate the MIME allowlist, force the album tag, stamp the
/// `#contributor` creator, and wake the owner's pipeline (thumbnails run under the owner).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, waker, share, meta))]
pub async fn public_complete_upload(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    waker: &RoutineHandle<Uuid>,
    share: &PublicShare,
    picture_id: Uuid,
    contributor_name: &str,
    mut meta: UploadMetadata,
) -> Result<crate::domain::picture::Picture, AppError> {
    if !share.allow_upload {
        return Err(AppError::Forbidden(
            "this album does not accept contributions".to_string(),
        ));
    }
    match meta.mime_type.as_deref() {
        Some(m) if is_ingestable_public_mime(m) => {}
        _ => {
            return Err(AppError::BadRequest(
                "unsupported file type (images and videos only)".to_string(),
            ));
        }
    }
    // Force the album tag; ignore any client-supplied tags/labels (a contributor can only tag into
    // this album).
    meta.initial_tags = Some(vec![share.tag_path.clone()]);
    meta.upload_label = None;
    let picture = pictures::complete_upload(
        db,
        cache,
        storage,
        settings,
        share.owner_id,
        picture_id,
        meta,
    )
    .await?;
    if let Some(creator) = contribution_creator(contributor_name) {
        PictureRepository::set_creator(db, share.owner_id, picture.id, Some(&creator)).await?;
    }
    waker.trigger_debounced(share.owner_id);
    PictureRepository::find_by_id(db, picture.id)
        .await?
        .ok_or(AppError::NotFound)
}

// ── Convert (authenticated visitor, §8) ─────────────────────────────────────────

/// Save a copy of a covered picture into the visitor's own library (feature 11 + 27 §8). Same-backend
/// only for now: the owner's picture must be a local row so the byte copy + provenance resolve.
/// Cross-instance save-a-copy is a follow-up (§10, the deepest escalation) — Subscribe instead.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, storage, settings, federation, waker), fields(visitor_id = %visitor_id))]
pub async fn public_save_copy(
    db: &PgPool,
    cache: &dyn Cache,
    storage: &dyn Storage,
    settings: &Settings,
    federation: &FederationClient,
    waker: &RoutineHandle<Uuid>,
    visitor_id: Uuid,
    owner_username: &str,
    owner_instance: &str,
    token: &str,
    picture_id: Uuid,
) -> Result<crate::domain::picture::Picture, AppError> {
    if find_local_user_id(cache, db, settings, owner_username, owner_instance)
        .await?
        .is_none()
    {
        return Err(AppError::Forbidden(
            "Saving a copy across instances is not yet supported. Convert to a private share instead."
                .to_string(),
        ));
    }
    let share = PublicShareRepository::find_by_token(db, token)
        .await?
        .filter(|s| s.is_accessible(Utc::now().naive_utc()) && s.allow_originals)
        .ok_or(AppError::NotFound)?;
    // You already own this album — no point copying your own pictures back into your library.
    if visitor_id == share.owner_id {
        return Err(AppError::BadRequest(
            "you already own this album".to_string(),
        ));
    }
    let source = PublicShareRepository::find_covered_picture(
        db,
        share.owner_id,
        &share.tag_path,
        picture_id,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    pictures::copy_covered_picture(
        db,
        cache,
        storage,
        settings,
        federation,
        waker,
        visitor_id,
        &source,
        owner_username,
        &settings.get(keys::GLOBAL_DOMAIN),
    )
    .await
}

/// Subscribe (Convert → derived share, §8): mint a derived `OutgoingShare` on the owner backend
/// (same-backend short-circuit or the `shares/public/claim` federation verb) and create the visitor's
/// active `IncomingShare`. The owner's pipeline announces coverage from there.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, pipeline_waker), fields(visitor_id = %visitor_id))]
pub async fn public_subscribe(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    pipeline_waker: &RoutineHandle<Uuid>,
    visitor_id: Uuid,
    visitor_username: &str,
    owner_username: &str,
    owner_instance: &str,
    token: &str,
) -> Result<PublicShareClaimResponse, AppError> {
    let global = settings.get(keys::GLOBAL_DOMAIN);
    let meta = if find_local_user_id(cache, db, settings, owner_username, owner_instance)
        .await?
        .is_some()
    {
        receive_public_claim(
            cache,
            db,
            pipeline_waker,
            settings,
            token,
            visitor_username,
            &global,
        )
        .await?
    } else {
        federation
            .send(
                visitor_username,
                owner_username,
                owner_instance,
                crate::clients::federation::models::PublicShareClaimRequest {
                    token: token.to_string(),
                    requester_username: visitor_username.to_string(),
                    requester_instance: global.clone(),
                },
            )
            .await?
    };

    // Create the visitor's active IncomingShare so the owner's picture announcement resolves it.
    let shared_tag = TagPath::shared_to_me(
        owner_username,
        owner_instance,
        &TagPath::from_ltree(meta.tag_path.clone()),
    );
    let incoming = IncomingShareRepository::create(
        db,
        visitor_id,
        owner_username,
        owner_instance,
        &meta.name,
        meta.message.as_deref(),
        meta.outgoing_share_id,
        meta.allow_share_back,
        meta.allow_exif_edit,
        meta.future,
        Some(shared_tag.as_ltree()),
        None,
    )
    .await?;
    IncomingShareRepository::set_status(db, incoming.id, ShareStatus::Active).await?;
    Ok(meta)
}

// ── Management (authenticated owner) ─────────────────────────────────────────────

/// The create/update payload (name/message/permissions + optional password + expiry).
#[derive(Debug)]
pub struct PublicShareInput {
    pub tag_path: String,
    pub name: String,
    pub message: Option<String>,
    pub password: Option<String>,
    pub expires_at: Option<NaiveDateTime>,
    pub allow_originals: bool,
    pub allow_upload: bool,
    pub allow_share_back: bool,
    pub conv_allow_exif_edit: bool,
    pub conv_future: bool,
}

#[tracing::instrument(skip(db, _settings, input), fields(owner_id = %owner_id))]
pub async fn create_public_share(
    db: &PgPool,
    _settings: &Settings,
    owner_id: Uuid,
    input: PublicShareInput,
) -> Result<PublicShare, AppError> {
    // A public share may cover received `SharedToMe/...` pictures (§10), so protected prefixes are
    // allowed for filtering — coverage is read-only.
    let tag_path = TagPath::parse(&input.tag_path, true).map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_share_name(&input.name).map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_share_message(input.message.as_deref())
        .map_err(AppError::BadRequest)?;
    let password_hash = hash_optional_password(input.password.as_deref())?;
    // Share-back needs originals (convert ⇒ a real share); forced on when uploads are allowed (§4).
    let allow_share_back = input.allow_originals && (input.allow_share_back || input.allow_upload);
    let token = PublicShare::generate_token();
    PublicShareRepository::create(
        db,
        owner_id,
        tag_path.as_ltree(),
        &input.name,
        input.message.as_deref(),
        &token,
        password_hash.as_deref(),
        input.expires_at,
        input.allow_originals,
        input.allow_upload,
        allow_share_back,
        input.conv_allow_exif_edit,
        input.conv_future,
    )
    .await
}

/// Update a public share. `keep_password` keeps the stored hash; otherwise `password` sets it (or
/// clears it when blank/absent). The `tag_path` is immutable (coverage is anchored to it).
#[tracing::instrument(skip(db, input), fields(owner_id = %owner_id, share_id = %share_id))]
pub async fn update_public_share(
    db: &PgPool,
    owner_id: Uuid,
    share_id: Uuid,
    input: PublicShareInput,
    keep_password: bool,
) -> Result<PublicShare, AppError> {
    let existing = PublicShareRepository::find_by_id(db, share_id)
        .await?
        .filter(|s| s.owner_id == owner_id)
        .ok_or(AppError::NotFound)?;
    crate::domain::validation::validate_share_name(&input.name).map_err(AppError::BadRequest)?;
    crate::domain::validation::validate_share_message(input.message.as_deref())
        .map_err(AppError::BadRequest)?;
    let password_hash = if keep_password {
        existing.password_hash.clone()
    } else {
        hash_optional_password(input.password.as_deref())?
    };
    let allow_share_back = input.allow_originals && (input.allow_share_back || input.allow_upload);
    PublicShareRepository::update(
        db,
        share_id,
        owner_id,
        &input.name,
        input.message.as_deref(),
        password_hash.as_deref(),
        input.expires_at,
        input.allow_originals,
        input.allow_upload,
        allow_share_back,
        input.conv_allow_exif_edit,
        input.conv_future,
    )
    .await?
    .ok_or(AppError::NotFound)
}

fn hash_optional_password(password: Option<&str>) -> Result<Option<String>, AppError> {
    match password.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Ok(Some(crypto::hash_password(p)?)),
        None => Ok(None),
    }
}

/// The result of a public-share revoke (with optional cascades).
#[derive(Debug, Serialize)]
pub struct PublicRevokeOutcome {
    pub revoked: bool,
    pub derived_revoked: u64,
    pub contributions_trashed: u64,
}

/// Revoke a public share (coverage cut is instant, §9). Optionally cascade-revoke the derived shares
/// minted from it, and/or trash its `#`-contributions.
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, federation, settings, task_queue, pipeline_waker), fields(owner_id = %owner_id, share_id = %share_id))]
pub async fn revoke_public_share(
    db: &PgPool,
    cache: &dyn Cache,
    federation: &FederationClient,
    settings: &Settings,
    task_queue: &RoutineHandle<UnannounceInput>,
    pipeline_waker: &RoutineHandle<Uuid>,
    owner_id: Uuid,
    owner_username: &str,
    share_id: Uuid,
    cascade_derived: bool,
    trash_contributions_flag: bool,
) -> Result<PublicRevokeOutcome, AppError> {
    let share = PublicShareRepository::find_by_id(db, share_id)
        .await?
        .filter(|s| s.owner_id == owner_id)
        .ok_or(AppError::NotFound)?;
    let revoked = PublicShareRepository::revoke(db, share_id, owner_id).await?;

    let mut derived_revoked = 0;
    if cascade_derived {
        for os in OutgoingShareRepository::find_derived_by_public_share(db, share_id).await? {
            revoke_outgoing_share(
                db,
                cache,
                federation,
                settings,
                task_queue,
                pipeline_waker,
                owner_id,
                owner_username,
                os.id,
            )
            .await?;
            derived_revoked += 1;
        }
    }

    let contributions_trashed = if trash_contributions_flag {
        trash_contributions(db, pipeline_waker, owner_id, &share.tag_path, None).await?
    } else {
        0
    };

    Ok(PublicRevokeOutcome {
        revoked,
        derived_revoked,
        contributions_trashed,
    })
}

/// Trash the owner's `#`-contributions under a tag (optionally a single contributor's, §9). Returns
/// the number trashed.
#[tracing::instrument(skip(db, pipeline_waker), fields(owner_id = %owner_id))]
pub async fn trash_contributions(
    db: &PgPool,
    pipeline_waker: &RoutineHandle<Uuid>,
    owner_id: Uuid,
    tag_path: &str,
    contributor: Option<&str>,
) -> Result<u64, AppError> {
    let ids = PublicShareRepository::contribution_ids(db, owner_id, tag_path, contributor).await?;
    if ids.is_empty() {
        return Ok(0);
    }
    let n = ids.len() as u64;
    let sel = ResolvedSelection::explicit(ids);
    pictures::batch_set_trashed_selection(db, pipeline_waker, owner_id, &sel, true, false).await?;
    Ok(n)
}

/// The derived-share + contribution counts shown alongside a share in the owner's management list.
#[tracing::instrument(skip(db), fields(owner_id = %owner_id))]
pub async fn list_public_shares_with_counts(
    db: &PgPool,
    owner_id: Uuid,
) -> Result<Vec<(PublicShare, i64, i64)>, AppError> {
    let shares = PublicShareRepository::list_by_owner(db, owner_id).await?;
    let mut out = Vec::with_capacity(shares.len());
    for s in shares {
        let derived = OutgoingShareRepository::find_derived_by_public_share(db, s.id)
            .await?
            .len() as i64;
        let contributions = PublicShareRepository::contribution_ids(db, owner_id, &s.tag_path, None)
            .await?
            .len() as i64;
        out.push((s, derived, contributions));
    }
    Ok(out)
}
