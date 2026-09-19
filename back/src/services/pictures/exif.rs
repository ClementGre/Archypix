use crate::clients::federation::FederationClient;
use crate::domain::picture::Picture;
use crate::infra::redis::Cache;
use archypix_common::routine::RoutineHandle;
use crate::infra::settings::keys;
use crate::repository::picture::{
    PictureRepository, ResolvedSelection,
};
use crate::services::users::find_local_user_id;
use archypix_common::error::{AppError, map_sqlx_error};
use archypix_common::job::{ExifField, FullExif};
use archypix_common::settings::Settings;
use sqlx::PgPool;
use uuid::Uuid;

/// Apply a recipient's local EXIF override to a **received** picture (09 §6.2): write the sparse
/// per-field key set into `local_exif_overrides`, re-materialise `exif_data` + promoted columns from
/// `merge(remote_exif_data, overrides)`, and fire the local `metadata` event (re-dirty + wake). DB
/// only — no `edit_picture` job, no file reconcile (the recipient does not own the file). `set`
/// fields claim the override; `empty` fields claim it as empty/`null` (10 §6.3); `clear` fields drop
/// the override (the owner's value flows through again). Returns the updated picture.
#[tracing::instrument(skip(db, cache, waker, set, empty, clear), fields(user_id = %user_id, picture_id = %picture_id))]
pub async fn override_received_exif(
    db: &PgPool,
    cache: &dyn Cache,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
    set: FullExif,
    empty: Vec<ExifField>,
    clear: Vec<ExifField>,
) -> Result<Picture, AppError> {
    use crate::domain::received_exif;

    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    if picture.is_owned() {
        return Err(AppError::BadRequest(
            "Local EXIF overrides apply to received pictures only; use /edit for owned pictures"
                .to_string(),
        ));
    }

    // Same normalisation + set-based merge as batch editing: `set` claims a field, `empty` claims it
    // as empty (null), `clear` drops the claim.
    let (empty, clear) = crate::domain::validation::validate_exif_edit(&set, empty, clear)
        .map_err(AppError::BadRequest)?;
    let (patch, clear_keys) = received_exif::override_patch(&set, &empty, &clear);
    let sel = ResolvedSelection::explicit(vec![picture_id]);
    PictureRepository::batch_apply_exif_received_local_selection(
        db,
        user_id,
        &sel,
        &patch,
        &clear_keys,
    )
    .await?;

    // A local `captured_at` override moves every covering tag's derived range (feature 34 §4).
    crate::services::tag_metadata::bust_cache(cache, user_id).await;
    waker.trigger(user_id);
    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Set a picture's creator credit (feature 26 §7). Owned → the authoritative `creator` (re-announced
/// via the pipeline). Received + `propose = false` → the recipient-local `creator_override`. Received
/// + `propose = true` → **phase 2**, not yet built (`403`). `value` null/blank resets to the owner
/// default (owned) or clears the override (received). Returns the updated picture.
#[tracing::instrument(skip(db, waker), fields(user_id = %user_id, picture_id = %picture_id, propose = propose))]
pub async fn set_picture_creator(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    picture_id: Uuid,
    value: Option<String>,
    propose: bool,
) -> Result<Picture, AppError> {
    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }

    // Normalise to the stored form: blank ⇒ None (reset/clear). Reject a forged system sigil (§3).
    let value = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if let Some(v) = value.as_deref() {
        crate::domain::picture::validate_manual_creator(v).map_err(AppError::BadRequest)?;
    }

    if picture.is_owned() {
        PictureRepository::set_creator(db, user_id, picture_id, value.as_deref()).await?;
        // Owned edit re-announces through the pipeline (updated_at bumped in the repo).
        waker.trigger_debounced(user_id);
    } else if propose {
        // Propose-to-owner (§7, phase 2) — mirrors feature 10's EXIF propose; not built yet.
        return Err(AppError::Forbidden(
            "Proposing a creator to the owner is not yet supported".to_string(),
        ));
    } else {
        PictureRepository::set_creator_override(db, user_id, picture_id, value.as_deref()).await?;
    }

    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

/// Result of a batch creator edit: the dry-run breakdown, or the applied owned/received counts.
pub enum CreatorBatchOutcome {
    DryRun(crate::services::aggregate::DryRun),
    Applied {
        affected: i64,
        edited: i64,
        local_override: i64,
    },
}

/// Batch-set the creator over a [`ResolvedSelection`] (feature 26 batch integration). Owned pictures
/// get the owner-authoritative `creator` (set-based; re-announces via the pipeline); received pictures
/// get the recipient-local `creator_override` (DB-only). `value = None`/blank resets/clears. Propose
/// mode is not offered in batch (phase 2). With `dry_run` returns the §6.1 breakdown without mutating.
#[tracing::instrument(skip(db, waker, sel), fields(user_id = %user_id, dry_run))]
pub async fn batch_set_creator_selection(
    db: &PgPool,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    sel: &crate::repository::picture::ResolvedSelection,
    value: Option<String>,
    dry_run: bool,
) -> Result<CreatorBatchOutcome, AppError> {
    // Normalise to the stored form (blank ⇒ reset/clear) + reject a forged system sigil (§3).
    let value = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if let Some(v) = value.as_deref() {
        crate::domain::picture::validate_manual_creator(v).map_err(AppError::BadRequest)?;
    }

    if dry_run {
        let affected = PictureRepository::count_selection(db, user_id, sel).await?;
        let edited = PictureRepository::count_owned_selection(db, user_id, sel).await?;
        return Ok(CreatorBatchOutcome::DryRun(
            crate::services::aggregate::DryRun {
                affected,
                edited: Some(edited),
                local_override: Some(affected - edited),
                ..Default::default()
            },
        ));
    }

    let mut tx = db.begin().await.map_err(map_sqlx_error)?;
    let edited = PictureRepository::batch_set_creator_selection(
        &mut *tx,
        user_id,
        sel,
        value.as_deref(),
        true,
    )
    .await? as i64;
    let local_override = PictureRepository::batch_set_creator_selection(
        &mut *tx,
        user_id,
        sel,
        value.as_deref(),
        false,
    )
    .await? as i64;
    tx.commit().await.map_err(map_sqlx_error)?;

    // Owned edits re-announce through the pipeline (updated_at bumped + re-dirtied). Debounced: a
    // batch produces a burst that should collapse into one run.
    if edited > 0 {
        waker.trigger_debounced(user_id);
    }

    Ok(CreatorBatchOutcome::Applied {
        affected: edited + local_override,
        edited,
        local_override,
    })
}

/// The twelve editable EXIF fields, used to enumerate which fields a `set`/`clear` delta touches.
const ALL_EXIF_FIELDS: [ExifField; 12] = {
    use crate::domain::job::ExifField::*;
    [
        CapturedAt,
        GpsLat,
        GpsLng,
        GpsAlt,
        Orientation,
        CameraBrand,
        CameraModel,
        FocalLengthMm,
        FNumber,
        IsoSpeed,
        ExposureTimeNum,
        ExposureTimeDen,
    ]
};

/// Propose an EXIF edit on a **received** picture to its owner (10 §4.1, `mode: "propose"`).
///
/// Requires an active incoming share that grants editing (`allow_exif_edit`); otherwise `403`. The
/// delta is sent to the owner's backend (same-backend owners are short-circuited to a direct service
/// call). On success the proposed fields are **dropped from `local_exif_overrides`** so the owner's
/// authoritative value — arriving via the owner's re-announce — is no longer shadowed (09 §6.2). The
/// authoritative change lands asynchronously (owner reconcile + re-announce), so the caller returns
/// `202`. Returns the locally-updated picture (overrides cleared for the proposed fields).
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(skip(db, cache, settings, federation, waker), fields(user_id = %user_id))]
pub async fn propose_received_exif(
    db: &PgPool,
    cache: &dyn Cache,
    settings: &Settings,
    federation: &FederationClient,
    waker: &RoutineHandle<Uuid>,
    user_id: Uuid,
    requester_username: &str,
    picture_id: Uuid,
    set: FullExif,
    clear: Vec<ExifField>,
) -> Result<Picture, AppError> {
    use crate::repository::share::IncomingShareRepository;

    let picture = PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if picture.local_user_id != user_id {
        return Err(AppError::NotFound);
    }
    if picture.is_owned() {
        return Err(AppError::BadRequest(
            "EXIF proposals apply to received pictures only; use /edit for owned pictures"
                .to_string(),
        ));
    }

    // Gate: an active incoming share covering this picture must grant EXIF editing (10 §4.1).
    if IncomingShareRepository::find_active_exif_editable_for_picture(db, picture_id, user_id)
        .await?
        .is_none()
    {
        return Err(AppError::Forbidden(
            "this share does not authorise EXIF editing; use a local override instead".to_string(),
        ));
    }

    let owner_username = picture.owner_username.clone().unwrap_or_default();
    let owner_instance = picture.owner_instance_domain.clone().unwrap_or_default();
    let remote_id = picture.remote_picture_id.clone().ok_or_else(|| {
        AppError::InternalServerError("received picture missing remote_picture_id".into())
    })?;

    // Deliver the proposal to the owner. Same-backend owner → direct service call (mirrors the
    // share-announce same-backend short-circuit); cross-instance → federation. The owner validates
    // the fields and re-checks the grant, so an invalid/forbidden proposal errors here *before* we
    // clear any local override.
    if find_local_user_id(cache, db, settings, &owner_username, &owner_instance)
        .await?
        .is_some()
    {
        crate::services::federation::receive_picture_edit_request(
            db,
            cache,
            waker,
            &remote_id,
            requester_username,
            &settings.get(keys::GLOBAL_DOMAIN),
            set.clone(),
            clear.clone(),
        )
        .await?;
    } else {
        federation
            .send(
                requester_username,
                &owner_username,
                &owner_instance,
                crate::clients::federation::models::PictureEditRequest {
                    picture_id: remote_id,
                    requester_username: requester_username.to_string(),
                    requester_instance: settings.get(keys::GLOBAL_DOMAIN).clone(),
                    set: set.clone(),
                    clear: clear.clone(),
                },
            )
            .await?;
    }

    // Escalate clears the per-field local override so the owner's applied value (arriving via the
    // re-announce) is authoritative (09 §6.2 / 10 §2). Drop every field the proposal touched.
    let mut touched: Vec<ExifField> = clear.clone();
    for f in ALL_EXIF_FIELDS {
        if set.has(f) && !touched.contains(&f) {
            touched.push(f);
        }
    }
    // Drop the touched fields from the override via the shared set-based merge (empty patch, the
    // touched keys as the clear set) — same path as a local override / batch edit.
    let (patch, clear_keys) =
        crate::domain::received_exif::override_patch(&FullExif::default(), &[], &touched);
    let sel = ResolvedSelection::explicit(vec![picture_id]);
    PictureRepository::batch_apply_exif_received_local_selection(
        db,
        user_id,
        &sel,
        &patch,
        &clear_keys,
    )
    .await?;
    waker.trigger(user_id);

    PictureRepository::find_by_id(db, picture_id)
        .await?
        .ok_or(AppError::NotFound)
}

