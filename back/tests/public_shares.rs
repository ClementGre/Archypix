//! Feature 27 (public shares) integration tests: coverage view, token/password gating, view-only
//! stripping, anonymous contribution (+ dedup-reject), and same-backend Subscribe + revoke cascade.

mod common;

use archypix_back::domain::auth::TokenType;
use archypix_back::domain::public_share::PublicShareStatus;
use archypix_back::domain::share::ShareStatus;
use archypix_back::infra::s3::Storage;
use archypix_back::infra::settings::{keys, test_settings_with};
use archypix_back::repository::picture::PictureRepository;
use archypix_back::repository::public_share::PublicShareRepository;
use archypix_back::repository::share::OutgoingShareRepository;
use archypix_back::services::pictures::{BatchUploadFile, PictureVariant};
use archypix_back::services::shares::public::{self, PublicShareInput};
use archypix_back::state::AppState;
use archypix_common::error::AppError;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn settings() -> Arc<Settings> {
    test_settings_with(&[])
}

fn input(tag: &str, name: &str) -> PublicShareInput {
    PublicShareInput {
        tag_path: tag.to_string(),
        name: name.to_string(),
        message: None,
        password: None,
        expires_at: None,
        allow_originals: true,
        allow_upload: false,
        allow_share_back: false,
        conv_allow_exif_edit: false,
        conv_future: true,
    }
}

async fn create(
    state: &AppState,
    owner: Uuid,
    input: PublicShareInput,
) -> archypix_back::domain::public_share::PublicShare {
    public::create_public_share(&state.db, &state.settings, owner, input)
        .await
        .unwrap()
}

// ── View ────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn coverage_view_lists_only_covered_pictures(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let in_cov = common::seed_picture_with_tag(&db, owner, "Photos.Travel.Alps").await;
    let _out = common::seed_picture_with_tag(&db, owner, "Photos.Other").await;

    let share = create(&state, owner, input("Photos.Travel", "Alps 2024")).await;

    let meta = public::public_meta(&db, &s, &share.token).await.unwrap();
    assert_eq!(meta.picture_count, 1);
    assert!(meta.owner_display.starts_with("@alice:"));

    let list = public::list_public_pictures(
        &db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &s,
        &state.federation,
        &share,
        1,
        50,
        PictureVariant::Medium,
    )
    .await
    .unwrap();
    let ids: Vec<Uuid> = list.items.iter().map(|i| i.id).collect();
    assert_eq!(ids, vec![in_cov], "only the covered picture is listed");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unknown_and_revoked_tokens_are_404(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let share = create(&state, owner, input("Photos", "All")).await;

    assert!(matches!(
        public::public_meta(&db, &s, "nope").await,
        Err(AppError::NotFound)
    ));

    PublicShareRepository::revoke(&db, share.id, owner)
        .await
        .unwrap();
    assert!(matches!(
        public::public_meta(&db, &s, &share.token).await,
        Err(AppError::NotFound)
    ));
    let refreshed = PublicShareRepository::find_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.status, PublicShareStatus::Revoked);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn password_gate_requires_valid_unlock_jwt(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let mut inp = input("Photos", "Locked");
    inp.password = Some("s3cret".to_string());
    let share = create(&state, owner, inp).await;

    // No bearer → Unauthorized.
    assert!(matches!(
        public::resolve_access(&db, &state.jwt, &s, &share.token, None).await,
        Err(AppError::Unauthorized(_))
    ));
    // Wrong password → Unauthorized.
    assert!(matches!(
        public::unlock(&db, &state.jwt, &s, &share.token, "wrong").await,
        Err(AppError::Unauthorized(_))
    ));
    // Correct password mints a JWT that unlocks access.
    let jwt = public::unlock(&db, &state.jwt, &s, &share.token, "s3cret")
        .await
        .unwrap();
    let claims = state.jwt.decode(&jwt, &s.get(keys::BACK_DOMAIN)).unwrap();
    assert_eq!(claims.token_type, TokenType::PublicShare);
    assert_eq!(claims.sub, share.id.to_string());
    assert!(
        public::resolve_access(&db, &state.jwt, &s, &share.token, Some(&jwt))
            .await
            .is_ok()
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn view_only_strips_captured_and_blocks_originals(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let pic = common::seed_picture_with_tag(&db, owner, "Photos.Travel").await;
    sqlx::query!(
        "UPDATE pictures SET captured_at = now() AT TIME ZONE 'utc' WHERE id = $1",
        pic
    )
    .execute(&db)
    .await
    .unwrap();

    let mut inp = input("Photos.Travel", "View only");
    inp.allow_originals = false; // ⇒ view-only gallery
    let share = create(&state, owner, inp).await;
    assert!(share.view_only());

    let list = public::list_public_pictures(
        &db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &s,
        &state.federation,
        &share,
        1,
        50,
        PictureVariant::Medium,
    )
    .await
    .unwrap();
    assert_eq!(list.items.len(), 1);
    assert!(
        list.items[0].captured_at.is_none(),
        "view-only strips captured_at"
    );

    // Original presign is forbidden on a view-only share.
    assert!(matches!(
        public::presign_public_picture(
            &db,
            state.cache.as_ref(),
            state.storage.as_ref(),
            &s,
            &state.federation,
            &share,
            pic,
            PictureVariant::Original,
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
}

// ── Contribution ──────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn anonymous_contribution_lands_owned_tagged_and_credited(db: PgPool) {
    let s = settings();
    let storage = Arc::new(common::MockStorage::new());
    let cache: Arc<dyn archypix_back::infra::redis::Cache> = Arc::new(common::InMemoryCache::new());
    let state = common::test_app_state_with_storage(db.clone(), &s, cache, storage.clone());
    let owner = common::seed_user(&db, "alice", "pw").await;

    let mut inp = input("Photos.Album", "Drop your photos");
    inp.allow_upload = true;
    let share = create(&state, owner, inp).await;

    // Presign a fresh contribution slot.
    let files = vec![BatchUploadFile {
        filename: "beach.jpg".to_string(),
        file_hash: Some("hash-new".to_string()),
        size: Some(3),
    }];
    let slots = public::public_upload_batch(
        &db,
        state.cache.as_ref(),
        storage.as_ref(),
        &s,
        &state.routines.pipeline,
        &share,
        "1.2.3.4",
        &files,
    )
    .await
    .unwrap();
    let pid = match &slots[0] {
        archypix_back::services::pictures::BatchUploadOutcome::New { picture_id, .. } => {
            *picture_id
        }
        _ => panic!("expected a fresh slot"),
    };

    // Simulate the client's S3 PUT into staging so complete finds the bytes.
    storage
        .put_object(
            &s.get(keys::S3_BUCKET_STAGING),
            &format!("staging/{owner}/{pid}"),
            vec![1, 2, 3],
            Some("image/jpeg"),
        )
        .await
        .unwrap();

    let meta = archypix_back::services::pictures::UploadMetadata {
        mime_type: Some("image/jpeg".to_string()),
        file_size: Some(3),
        file_hash: Some("hash-new".to_string()),
        width: Some(10),
        height: Some(10),
        exif_data: None,
        captured_at: None,
        original_file_created_at: None,
        initial_tags: None,
        upload_label: None,
        defer_pipeline: true,
    };
    let picture = public::public_complete_upload(
        &db,
        state.cache.as_ref(),
        storage.as_ref(),
        &s,
        &state.routines.pipeline,
        &share,
        pid,
        "Bob Contributor",
        meta,
    )
    .await
    .unwrap();

    assert!(
        picture.is_owned(),
        "contribution is owned by the album owner"
    );
    assert_eq!(picture.local_user_id, owner);
    assert_eq!(picture.creator.as_deref(), Some("#Bob Contributor"));
    // Tagged into the album.
    let tags: Vec<String> =
        sqlx::query_scalar("SELECT tag_path::text FROM tags WHERE picture_id = $1")
            .bind(picture.id)
            .fetch_all(&db)
            .await
            .unwrap();
    assert!(tags.iter().any(|t| t == "Photos.Album"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn contribution_dedup_rejects_hash_collision(db: PgPool) {
    let s = settings();
    let storage = Arc::new(common::MockStorage::new());
    let cache: Arc<dyn archypix_back::infra::redis::Cache> = Arc::new(common::InMemoryCache::new());
    let state = common::test_app_state_with_storage(db.clone(), &s, cache, storage.clone());
    let owner = common::seed_user(&db, "alice", "pw").await;

    // An existing owned picture with a known hash.
    let existing = common::seed_picture(&db, owner).await;
    PictureRepository::set_file_hash(&db, existing, "dup-hash", Some(100))
        .await
        .unwrap();

    let mut inp = input("Photos.Album", "Drop box");
    inp.allow_upload = true;
    let share = create(&state, owner, inp).await;

    let files = vec![BatchUploadFile {
        filename: "same.jpg".to_string(),
        file_hash: Some("dup-hash".to_string()),
        size: Some(100),
    }];
    let slots = public::public_upload_batch(
        &db,
        state.cache.as_ref(),
        storage.as_ref(),
        &s,
        &state.routines.pipeline,
        &share,
        "1.2.3.4",
        &files,
    )
    .await
    .unwrap();
    assert!(
        matches!(
            slots[0],
            archypix_back::services::pictures::BatchUploadOutcome::Duplicate { .. }
        ),
        "a byte-dupe of the owner's picture is rejected, not stored"
    );
    // The existing picture must NOT be tagged into the album (no auto-tagging of dups).
    let tagged: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tags WHERE picture_id = $1 AND tag_path <@ 'Photos.Album'::ltree)",
    )
        .bind(existing)
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(
        !tagged,
        "dedup-reject must not tag the owner's existing picture into the album"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn upload_rejected_when_uploads_disabled(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let share = create(&state, owner, input("Photos", "No uploads")).await; // allow_upload = false

    let files = vec![BatchUploadFile {
        filename: "x.jpg".to_string(),
        file_hash: None,
        size: Some(1),
    }];
    assert!(matches!(
        public::public_upload_batch(
            &db,
            state.cache.as_ref(),
            state.storage.as_ref(),
            &s,
            &state.routines.pipeline,
            &share,
            "1.2.3.4",
            &files,
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
}

// ── Convert (same-backend Subscribe) + revocation ───────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn subscribe_same_backend_mints_derived_share(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let visitor = common::seed_user(&db, "bob", "pw").await;
    common::seed_picture_with_tag(&db, owner, "Photos.Travel").await;

    let mut inp = input("Photos.Travel", "Convertible");
    inp.allow_share_back = true;
    let share = create(&state, owner, inp).await;

    let meta = public::public_subscribe(
        &db,
        state.cache.as_ref(),
        &state.federation,
        &s,
        &state.routines.pipeline,
        visitor,
        "bob",
        "alice",
        "test.com",
        &share.token,
    )
    .await
    .unwrap();

    // A derived OutgoingShare (owner → visitor) exists, provenance-stamped, awaiting first announce.
    let derived = OutgoingShareRepository::find_derived_by_public_share(&db, share.id)
        .await
        .unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].id, meta.outgoing_share_id);
    assert_eq!(derived[0].recipient_username, "bob");
    assert_eq!(derived[0].status, ShareStatus::PendingFirstAnnouncement);
    assert!(derived[0].allow_share_back);

    // The visitor holds an active IncomingShare against it.
    let incoming = sqlx::query_scalar::<_, String>(
        r#"SELECT status::text FROM incoming_shares
           WHERE recipient_id = $1 AND outgoing_share_id = $2"#,
    )
    .bind(visitor)
    .bind(meta.outgoing_share_id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(incoming, "active");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn revoke_cascade_revokes_derived_shares(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let visitor = common::seed_user(&db, "bob", "pw").await;
    common::seed_picture_with_tag(&db, owner, "Photos.Travel").await;

    let share = create(&state, owner, input("Photos.Travel", "Convertible")).await;
    public::public_subscribe(
        &db,
        state.cache.as_ref(),
        &state.federation,
        &s,
        &state.routines.pipeline,
        visitor,
        "bob",
        "alice",
        "test.com",
        &share.token,
    )
    .await
    .unwrap();

    let outcome = public::revoke_public_share(
        &db,
        state.cache.as_ref(),
        &state.federation,
        &s,
        &state.routines.unannounce,
        &state.routines.pipeline,
        owner,
        "alice",
        share.id,
        true,  // cascade derived
        false, // keep contributions
    )
    .await
    .unwrap();
    assert!(outcome.revoked);
    assert_eq!(outcome.derived_revoked, 1);

    // The derived share is now revoked (excluded from the non-revoked lookup).
    assert!(
        OutgoingShareRepository::find_derived_by_public_share(&db, share.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Regression: a cross-instance visitor has no user row on the owner's backend. The owner-side
/// `receive_public_claim` must resolve the *federated* identity (not a bare username lookup) so the
/// remote requester isn't 404'd — it mints the derived share for a recipient on another instance.
#[sqlx::test(migrator = "MIGRATOR")]
async fn claim_from_remote_visitor_mints_derived_share(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    common::seed_picture_with_tag(&db, owner, "Photos.Travel").await;
    let share = create(&state, owner, input("Photos.Travel", "Convertible")).await;

    // "carol" exists on no local backend; her instance is not this backend's global domain.
    let meta = archypix_back::services::federation::receive_public_claim(
        state.cache.as_ref(),
        &db,
        &state.routines.pipeline,
        &s,
        &share.token,
        "carol",
        "remote.com",
    )
    .await
    .expect("a remote visitor must be able to claim");

    let derived = OutgoingShareRepository::find_derived_by_public_share(&db, share.id)
        .await
        .unwrap();
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].id, meta.outgoing_share_id);
    assert_eq!(derived[0].recipient_username, "carol");
    assert_eq!(derived[0].recipient_instance, "remote.com");
}

/// The album owner cannot subscribe to their own public share (loop prevention).
#[sqlx::test(migrator = "MIGRATOR")]
async fn claim_by_owner_is_rejected(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    common::seed_picture_with_tag(&db, owner, "Photos.Travel").await;
    let share = create(&state, owner, input("Photos.Travel", "Convertible")).await;

    let err = archypix_back::services::federation::receive_public_claim(
        state.cache.as_ref(),
        &db,
        &state.routines.pipeline,
        &s,
        &share.token,
        "alice",
        &s.get(keys::GLOBAL_DOMAIN),
    )
    .await
    .expect_err("subscribing to your own album must be rejected");
    assert!(matches!(err, AppError::BadRequest(_)));
}

/// The album owner cannot save a copy of their own pictures back into their own library.
#[sqlx::test(migrator = "MIGRATOR")]
async fn save_copy_from_own_share_is_rejected(db: PgPool) {
    let s = settings();
    let state = common::test_app_state(db.clone(), &s);
    let owner = common::seed_user(&db, "alice", "pw").await;
    let pic = common::seed_picture_with_tag(&db, owner, "Photos.Travel").await;
    let share = create(&state, owner, input("Photos.Travel", "Convertible")).await;

    let err = public::public_save_copy(
        &db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &s,
        &state.federation,
        &state.routines.pipeline,
        owner, // visitor is the owner
        "alice",
        &s.get(keys::GLOBAL_DOMAIN),
        &share.token,
        pic,
    )
    .await
    .expect_err("saving a copy from your own album must be rejected");
    assert!(matches!(err, AppError::BadRequest(_)));
}
