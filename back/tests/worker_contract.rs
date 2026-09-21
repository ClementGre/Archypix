//! Worker-backend HTTP contract test.
//!
//! Spins up the full Axum router (with a real Postgres DB, InMemoryCache, and
//! MockStorage) and drives the entire worker API via `tower::ServiceExt::oneshot`.
//! No network binding is needed — requests go through the in-process service.
//!
//! Covered scenarios:
//!  1. Claim a `gen_thumbnail` job  → correct job_id, claim_token, presigned URLs
//!  2. Second claim attempt         → `null` (job already claimed)
//!  3. Complete with correct token  → `204 No Content`; job row is `completed`
//!  4. Stale complete replay        → `409 Conflict`
//!  5. Fail path (permanent)        → `204`; job row is `failed`
//!  6. Fail with wrong token        → `409 Conflict`

mod common;

use archypix_back::domain::auth::TokenType;
use archypix_back::domain::job::{FullExif, JobStatus};
use archypix_back::domain::picture::ExifSyncStatus;
use archypix_back::infra::crypto::JwtService;
use archypix_back::routines::RoutineHandle;
use archypix_back::infra::settings::{keys, test_settings_with};
use archypix_back::repository::job::JobRepository;
use archypix_back::repository::picture::PictureRepository;
use archypix_back::services::jobs::enqueue_thumbnail_job;
use archypix_common::settings::Settings;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── helpers ───────────────────────────────────────────────────────────────────

/// Issue a short-lived worker JWT for `worker_id`.
fn worker_token(settings: &Settings) -> String {
    let jwt = JwtService::new(
        &settings.get(keys::WORKER_JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );
    jwt.issue(
        "test-worker-01",
        None,
        &settings.get(keys::GLOBAL_DOMAIN),
        TokenType::Worker,
        false,
        &settings.get(keys::BACK_DOMAIN),
        3600,
    )
    .unwrap()
}

/// Build a GET request with an optional JSON body to `/api/worker/…`.
fn get(uri: &str, bearer: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .body(Body::empty())
        .unwrap()
}

/// Build a POST request with a JSON body to `/api/worker/…`.
fn post_json(uri: &str, bearer: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

/// Collect and parse the response body as JSON.
async fn json_body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ── contract tests ────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn worker_claim_complete_cycle(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);

    // Seed: user + picture + pending gen_thumbnail job
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let job = enqueue_thumbnail_job(&db, alice_id, pic_id, true, None)
        .await
        .unwrap();

    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    // ── 1. Claim the job ──────────────────────────────────────────────────────
    let resp = app
        .clone()
        .oneshot(get("/api/worker/jobs/next?types=gen_thumbnail", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "claim must succeed");

    let body = json_body(resp).await;
    assert_eq!(body["job_id"].as_str().unwrap(), job.id.to_string());
    let claim_token: Uuid = body["claim_token"]
        .as_str()
        .unwrap()
        .parse()
        .expect("claim_token must be a UUID");
    // MockStorage returns non-empty presigned URLs
    assert!(
        body["presigned_read"]
            .as_str()
            .unwrap_or("")
            .starts_with("http://"),
        "presigned_read must be populated"
    );

    // ── 2. Second claim on same job type → null (already processing) ──────────
    let resp2 = app
        .clone()
        .oneshot(get("/api/worker/jobs/next?types=gen_thumbnail", &token))
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = json_body(resp2).await;
    assert!(
        body2.is_null(),
        "second claim must return null — job already processing"
    );

    // ── 3. Complete with correct claim_token → 204 ────────────────────────────
    // No `exif` in the body (e.g. a GIF) — decoded width/height must still be applied.
    let complete_body = serde_json::json!({
        "claim_token": claim_token,
        "outcome": "done",
        "job": "gen_thumbnail",
        "thumbnails_generated": true,
        "file_hash": "abc123deadbeef",
        "file_size": 204800,
        "width": 800,
        "height": 600
    });
    let resp3 = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/respond", job.id),
            &token,
            &complete_body,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp3.status(),
        StatusCode::NO_CONTENT,
        "complete must return 204"
    );

    // DB: job must be completed
    let completed = JobRepository::find_by_id(&db, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.status, JobStatus::Completed);

    // DB: decoded dimensions applied even without an `exif` payload.
    let (w, h): (Option<i32>, Option<i32>) =
        sqlx::query_as("SELECT width, height FROM pictures WHERE id = $1")
            .bind(pic_id)
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        (w, h),
        (Some(800), Some(600)),
        "width/height set from decoded dims (no EXIF)"
    );

    // ── 4. Replay completion → 409 (claim_token already consumed) ────────────
    let resp4 = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/respond", job.id),
            &token,
            &complete_body,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp4.status(),
        StatusCode::CONFLICT,
        "stale complete must return 409"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn worker_fail_permanent_marks_job_failed(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let job = enqueue_thumbnail_job(&db, alice_id, pic_id, true, None)
        .await
        .unwrap();

    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    // Claim
    let resp = app
        .clone()
        .oneshot(get("/api/worker/jobs/next", &token))
        .await
        .unwrap();
    let body = json_body(resp).await;
    let claim_token: Uuid = body["claim_token"].as_str().unwrap().parse().unwrap();

    // Fail with permanent=true
    let fail_body = serde_json::json!({
        "claim_token": claim_token,
        "outcome": "failed",
        "job": "gen_thumbnail",
        "error": "unsupported image format",
    });
    let resp_fail = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/respond", job.id),
            &token,
            &fail_body,
        ))
        .await
        .unwrap();
    assert_eq!(resp_fail.status(), StatusCode::NO_CONTENT);

    let failed = JobRepository::find_by_id(&db, job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(
        failed.error_message.as_deref(),
        Some("unsupported image format")
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn worker_fail_wrong_token_returns_conflict(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let job = enqueue_thumbnail_job(&db, alice_id, pic_id, false, None)
        .await
        .unwrap();

    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    // Claim (discard the real claim_token)
    app.clone()
        .oneshot(get("/api/worker/jobs/next", &token))
        .await
        .unwrap();

    // Fail with a random (wrong) claim_token
    let wrong_token = Uuid::new_v4();
    let fail_body = serde_json::json!({
        "claim_token": wrong_token,
        "outcome": "retry",
        "job": "gen_thumbnail",
        "error": "some error",
    });
    let resp = app
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/respond", job.id),
            &token,
            &fail_body,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "wrong claim_token must return 409"
    );
}

// ── Feature 31: state-based EXIF reconcile ────────────────────────────────────

/// Seed an owned, extracted JPEG carrying `gps_lat`, and enqueue an EXIF reconcile for it.
async fn seed_exif_edit(db: &PgPool, user_id: Uuid) -> (Uuid, Uuid) {
    let pic_id = common::seed_picture(db, user_id).await;
    sqlx::query!(
        "UPDATE pictures
         SET mime_type = 'image/jpeg', thumbnails_generated_at = (now() AT TIME ZONE 'utc')
         WHERE id = $1",
        pic_id,
    )
    .execute(db)
    .await
    .unwrap();
    let waker = RoutineHandle::<Uuid>::disconnected();
    let outcome = archypix_back::services::jobs::edit_pictures_exif(
        db,
        &common::InMemoryCache::new(),
        &waker,
        user_id,
        &[pic_id],
        FullExif {
            gps_lat: Some(48.8566),
            gps_lng: Some(2.3522),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();
    (pic_id, outcome.jobs[0])
}

/// Claim the next `edit_picture` job, returning its `claim_token` and the config handed out.
async fn claim_edit(app: &axum::Router, token: &str) -> (Uuid, Value) {
    let resp = app
        .clone()
        .oneshot(get("/api/worker/jobs/next?types=edit_picture", token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = json_body(resp).await;
    let claim_token: Uuid = body["claim_token"].as_str().unwrap().parse().unwrap();
    (claim_token, body["config"].clone())
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_claim_binds_the_live_target_and_persists_it(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    // A further edit lands before the worker claims: the claim must hand out the *latest* state.
    let waker = RoutineHandle::<Uuid>::disconnected();
    archypix_back::services::jobs::edit_pictures_exif(
        &db,
        &common::InMemoryCache::new(),
        &waker,
        alice_id,
        &[pic_id],
        FullExif {
            orientation: Some(6),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();

    let (_claim_token, config) = claim_edit(&app, &token).await;
    let target = &config["exif"]["target"];
    assert_eq!(target["gps_lat"].as_f64(), Some(48.8566));
    assert_eq!(target["orientation"].as_i64(), Some(6), "latest DB state");

    // The bound target is persisted on the job row so completion knows what was written.
    let job = JobRepository::find_by_id(&db, job_id)
        .await
        .unwrap()
        .unwrap();
    let stored = job.config.0;
    assert_eq!(stored["exif"]["target"]["orientation"].as_i64(), Some(6));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_completion_records_file_exif_and_syncs(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let (claim_token, _config) = claim_edit(&app, &token).await;

    // The read-back never returns the exact f64 it was handed — EXIF stores GPS as rationals.
    // Convergence is decided against the written target, so this must still land on `synced`.
    let complete_body = serde_json::json!({
        "claim_token": claim_token,
        "outcome": "done",
        "job": "edit_picture",
        "thumbnails_generated": false,
        "exif": {"extracted": {"gps_lat": 48.856599999, "gps_lng": 2.3522000001}},
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &complete_body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Synced);
    let file_exif = picture.file_exif.expect("read-back snapshot recorded");
    assert_eq!(file_exif.0.gps_lat, Some(48.856599999));
    assert_eq!(
        picture.gps_lat,
        Some(48.8566),
        "the DB keeps its own precise value"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn an_edit_during_processing_is_requeued_for_the_drain(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let (claim_token, _config) = claim_edit(&app, &token).await;

    // The user edits again while the worker is writing the claimed target.
    let waker = RoutineHandle::<Uuid>::disconnected();
    archypix_back::services::jobs::edit_pictures_exif(
        &db,
        &common::InMemoryCache::new(),
        &waker,
        alice_id,
        &[pic_id],
        FullExif {
            orientation: Some(3),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();

    let complete_body = serde_json::json!({
        "claim_token": claim_token,
        "outcome": "done",
        "job": "edit_picture",
        "thumbnails_generated": false,
        "exif": {"extracted": {"gps_lat": 48.8566, "gps_lng": 2.3522}},
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &complete_body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        picture.exif_sync_status,
        ExifSyncStatus::PendingJobCreation,
        "the newer edit must not be lost — the drain owns the follow-up"
    );

    // And the drain does create it.
    let created = archypix_back::services::jobs::create_deferred_exif_jobs(&db, 10)
        .await
        .unwrap();
    assert_eq!(created, 1);
    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Pending);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_permanent_failure_marks_write_failed(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let (claim_token, _config) = claim_edit(&app, &token).await;
    let fail_body = serde_json::json!({
        "claim_token": claim_token,
        "outcome": "failed",
        "job": "edit_picture",
        "error": "upload failed",
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &fail_body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::WriteFailed);
    assert_eq!(
        picture.gps_lat,
        Some(48.8566),
        "the DB edit is kept — the user decides whether to retry or revert"
    );
}

/// §4.4/§4.5: the write path's failure carries a verdict in the same vocabulary as the read path,
/// and each variant lands its own state. `Failed` is about these bytes, `UnsupportedMime` about the
/// format, and anything else opened fine so the write merely did not land.
#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_write_failure_verdicts_land_their_states(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    for (exif, expected) in [
        (serde_json::json!("failed"), ExifSyncStatus::UnsupportedFile),
        (
            serde_json::json!("unsupported_mime"),
            ExifSyncStatus::UnsupportedMime,
        ),
        (
            serde_json::json!("not_attempted"),
            ExifSyncStatus::WriteFailed,
        ),
    ] {
        let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
        let (claim_token, _config) = claim_edit(&app, &token).await;
        let resp = app
            .clone()
            .oneshot(post_json(
                &format!("/api/worker/jobs/{job_id}/respond"),
                &token,
                &serde_json::json!({
                    "claim_token": claim_token,
                    "outcome": "failed",
                    "job": "edit_picture",
                    "error": "write did not land",
                    "exif": exif,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let picture = PictureRepository::find_by_id(&db, pic_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(picture.exif_sync_status, expected, "verdict {exif}");
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn extraction_overwrites_pending_edits_and_records_the_file(db: PgPool) {
    // §5: a WebDAV overwrite re-extracts; the new file becomes the source of truth.
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, _job_id) = seed_exif_edit(&db, alice_id).await;
    let extraction = enqueue_thumbnail_job(&db, alice_id, pic_id, true, None)
        .await
        .unwrap();
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let resp = app
        .clone()
        .oneshot(get("/api/worker/jobs/next?types=gen_thumbnail", &token))
        .await
        .unwrap();
    let claim_token: Uuid = json_body(resp).await["claim_token"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let complete_body = serde_json::json!({
        "claim_token": claim_token,
        "outcome": "done",
        "job": "gen_thumbnail",
        "thumbnails_generated": true,
        "exif": {"extracted": {"gps_lat": 1.0, "gps_lng": 2.0, "camera_brand": "Nikon"}},
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/respond", extraction.id),
            &token,
            &complete_body,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Synced);
    assert_eq!(picture.gps_lat, Some(1.0), "the file wins");
    assert_eq!(picture.exif_data.0.camera_brand.as_deref(), Some("Nikon"));
    assert_eq!(
        picture.file_exif.map(|f| f.0.gps_lat),
        Some(Some(1.0)),
        "file_exif tracks the newly extracted state"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_write_back_moves_file_modified_at_only_when_the_hash_changes(db: PgPool) {
    // Feature 32: an EXIF write-through rewrites the file, so the WebDAV last-modified must move
    // with the ETag. A completion reporting the *same* hash (retry, no-op re-write) must not.
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    sqlx::query!("UPDATE pictures SET file_hash = 'h0' WHERE id = $1", pic_id)
        .execute(&db)
        .await
        .unwrap();
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let mtime = |db: PgPool| async move {
        PictureRepository::find_by_id(&db, pic_id)
            .await
            .unwrap()
            .unwrap()
            .file_modified_at
    };
    let before = mtime(db.clone()).await;

    let complete = |job: Uuid, claim: Uuid, hash: &str| {
        let body = serde_json::json!({
            "claim_token": claim,
            "outcome": "done",
            "job": "edit_picture",
            "thumbnails_generated": false,
            "file_hash": hash,
            "exif": {"extracted": {"gps_lat": 48.8566, "gps_lng": 2.3522}},
        });
        let app = app.clone();
        let token = token.clone();
        async move {
            let resp = app
                .oneshot(post_json(
                    &format!("/api/worker/jobs/{job}/respond"),
                    &token,
                    &body,
                ))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::NO_CONTENT);
        }
    };

    let (claim_token, _) = claim_edit(&app, &token).await;
    complete(job_id, claim_token, "h1").await;
    let after_write = mtime(db.clone()).await;
    assert!(
        after_write > before,
        "the worker rewrote the file (new hash) — the last-modified must move"
    );

    // A second reconcile whose read-back reports the same bytes leaves it alone.
    let waker = RoutineHandle::<Uuid>::disconnected();
    let outcome = archypix_back::services::jobs::edit_pictures_exif(
        &db,
        &common::InMemoryCache::new(),
        &waker,
        alice_id,
        &[pic_id],
        FullExif {
            orientation: Some(6),
            ..Default::default()
        },
        vec![],
    )
    .await
    .unwrap();
    let (claim_token, _) = claim_edit(&app, &token).await;
    complete(outcome.jobs[0], claim_token, "h1").await;
    assert_eq!(
        mtime(db.clone()).await,
        after_write,
        "an unchanged hash must not move the last-modified"
    );
}

// ── Feature 33: the read direction's outcomes ────────────────────────────────

/// Seed a picture mid-extraction: a `gen_thumbnail(is_initial)` job plus the `extracting` status the
/// insert paths stamp. Returns `(picture_id, job_id)`.
async fn seed_extraction(db: &PgPool, user_id: Uuid, mime: &str) -> (Uuid, Uuid) {
    let pic_id = common::seed_picture(db, user_id).await;
    sqlx::query!(
        "UPDATE pictures SET mime_type = $2 WHERE id = $1",
        pic_id,
        mime,
    )
    .execute(db)
    .await
    .unwrap();
    PictureRepository::set_exif_sync_status(db, pic_id, ExifSyncStatus::Extracting)
        .await
        .unwrap();
    let job = enqueue_thumbnail_job(db, user_id, pic_id, true, None)
        .await
        .unwrap();
    (pic_id, job.id)
}

/// Claim the next `gen_thumbnail` job, returning its `claim_token`.
async fn claim_thumbnail(app: &axum::Router, token: &str) -> Uuid {
    let resp = app
        .clone()
        .oneshot(get("/api/worker/jobs/next?types=gen_thumbnail", token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    json_body(resp).await["claim_token"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// §6.1: each extraction outcome lands its own state, and none of them is the schema's opinion.
#[sqlx::test(migrator = "MIGRATOR")]
async fn each_extraction_outcome_lands_its_state(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    for (outcome, expected) in [
        (
            serde_json::json!({"extracted": {"gps_lat": 1.0}}),
            ExifSyncStatus::Synced,
        ),
        (
            serde_json::json!("unsupported_mime"),
            ExifSyncStatus::UnsupportedMime,
        ),
        (serde_json::json!("failed"), ExifSyncStatus::UnsupportedFile),
    ] {
        let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;
        let claim_token = claim_thumbnail(&app, &token).await;
        let resp = app
            .clone()
            .oneshot(post_json(
                &format!("/api/worker/jobs/{job_id}/respond"),
                &token,
                &serde_json::json!({
                    "claim_token": claim_token,
                    "outcome": "done",
                    "job": "gen_thumbnail",
                    "thumbnails_generated": true,
                    "exif": outcome,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let picture = PictureRepository::find_by_id(&db, pic_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(picture.exif_sync_status, expected, "outcome {outcome}");
    }
}

/// Feature 36 §5 end to end: the `synced` sweep's job is claimed with no thumbnail write URLs, and
/// its read lands the newly-extracted field while the stored thumbnails and dimensions stay put.
#[sqlx::test(migrator = "MIGRATOR")]
async fn a_metadata_only_re_extraction_keeps_the_thumbnails(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));
    let pic_id = common::seed_picture(&db, alice_id).await;
    sqlx::query!(
        "UPDATE pictures SET mime_type = 'image/jpeg', width = 4000, height = 3000, file_exif = '{}'::jsonb,
                             thumbnails_generated_at = '2024-01-01T00:00:00', blurhash = 'LKO2?U%2Tw=w'
         WHERE id = $1",
        pic_id,
    )
    .execute(&db)
    .await
    .unwrap();
    archypix_back::services::jobs::recheck_exif_batch(
        &db,
        archypix_back::domain::routine::RecheckScope::Synced,
        None,
        Uuid::new_v4(),
        None,
        10,
    )
    .await
    .unwrap();

    let resp = app
        .clone()
        .oneshot(get("/api/worker/jobs/next?types=gen_thumbnail", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let claim = json_body(resp).await;
    let writes = &claim["presigned_writes"];
    for variant in ["small", "medium", "large"] {
        assert!(writes.get(variant).is_none(), "no {variant} thumbnail URL: {writes}");
    }

    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/respond", claim["job_id"].as_str().unwrap()),
            &token,
            &serde_json::json!({
                "claim_token": claim["claim_token"],
                "outcome": "done",
                "job": "gen_thumbnail",
                "thumbnails_generated": false,
                "exif": {"extracted": {"gps_lat": 48.85, "gps_lng": 2.29, "gps_accuracy_m": 4.74}},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id).await.unwrap().unwrap();
    assert_eq!(picture.gps_accuracy_m, Some(4.74));
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Synced);
    assert_eq!(picture.blurhash.as_deref(), Some("LKO2?U%2Tw=w"));
    assert_eq!((picture.width, picture.height), (Some(4000), Some(3000)));
    assert_eq!(
        picture.thumbnails_generated_at.map(|t| t.to_string()),
        Some("2024-01-01 00:00:00".into()),
    );
}

/// §6.5: a permanently failed extraction job never reaches `complete_job` and the watchdog only
/// rescues budget exhaustion, so `fail_job` has to settle the row — otherwise it stays `extracting`
/// forever with no job left to move it, and every edit is refused.
#[sqlx::test(migrator = "MIGRATOR")]
async fn each_failed_extraction_outcome_settles_the_row(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    for (outcome, expected) in [
        (
            serde_json::json!({"extracted": {"gps_lat": 1.0}}),
            ExifSyncStatus::Synced,
        ),
        (
            serde_json::json!("unsupported_mime"),
            ExifSyncStatus::UnsupportedMime,
        ),
        (serde_json::json!("failed"), ExifSyncStatus::UnsupportedFile),
        // The job died before it got a verdict — an absence of one, not a verdict.
        (
            serde_json::json!("not_attempted"),
            ExifSyncStatus::ExtractFailed,
        ),
    ] {
        let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;
        let claim_token = claim_thumbnail(&app, &token).await;
        let resp = app
            .clone()
            .oneshot(post_json(
                &format!("/api/worker/jobs/{job_id}/respond"),
                &token,
                &serde_json::json!({
                    "claim_token": claim_token,
                    "outcome": "failed",
                    "job": "gen_thumbnail",
                    "error": "thumbnailer: codec failure",
                    "exif": outcome,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let picture = PictureRepository::find_by_id(&db, pic_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(picture.exif_sync_status, expected, "outcome {outcome}");
    }
}

/// The read direction succeeded even though the job did not: the EXIF lands, but
/// `thumbnails_generated_at` must not — the thumbnails are exactly what failed, and stamping it
/// would serve a missing thumbnail and hide the row from the regeneration sweep.
#[sqlx::test(migrator = "MIGRATOR")]
async fn a_failed_thumbnail_job_still_records_what_it_read(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let claim_token = claim_thumbnail(&app, &token).await;
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &serde_json::json!({
                "claim_token": claim_token,
                "outcome": "failed",
                "job": "gen_thumbnail",
                "error": "thumbnailer: codec failure",
                "exif": {"extracted": {"gps_lat": 48.8566, "gps_lng": 2.3522}},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Synced);
    assert_eq!(picture.gps_lat, Some(48.8566));
    assert!(
        picture.file_exif.is_some(),
        "the file snapshot lands, so Revert-to-file works on a picture whose thumbnails failed"
    );
    assert!(
        picture.thumbnails_generated_at.is_none(),
        "the thumbnails failed — the row must stay visible to the regeneration sweep"
    );
}

/// A retriable failure is not a verdict: the job will be retried, so the row stays `extracting`.
#[sqlx::test(migrator = "MIGRATOR")]
async fn a_retriable_job_failure_leaves_the_row_extracting(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let claim_token = claim_thumbnail(&app, &token).await;
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &serde_json::json!({
                "claim_token": claim_token,
                "outcome": "retry",
                "job": "gen_thumbnail",
                "error": "connection reset",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Extracting);
}

/// A retriable extraction failure never completes the job, so it must leave the status alone —
/// the retry budget and the watchdog own the row from here (§6.1).
#[sqlx::test(migrator = "MIGRATOR")]
async fn a_retriable_extraction_failure_leaves_the_status_alone(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));
    let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;

    let claim_token = claim_thumbnail(&app, &token).await;
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &serde_json::json!({
                "claim_token": claim_token,
                "outcome": "retry",
                "job": "gen_thumbnail",
                "error": "connection reset while downloading",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Extracting);
}

/// §4.2: an extraction whose retry budget is exhausted never reaches `fail_job`, so the watchdog
/// records the absence of a verdict itself.
#[sqlx::test(migrator = "MIGRATOR")]
async fn watchdog_exhaustion_marks_the_extraction_failed(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;
    // A claimed job whose next retry is its last, stalled well past the processing timeout.
    sqlx::query!(
        "UPDATE jobs
         SET status = 'processing', retry_count = max_retries - 1,
             started_at = (now() AT TIME ZONE 'utc') - INTERVAL '1 day'
         WHERE id = $1",
        job_id,
    )
    .execute(&db)
    .await
    .unwrap();

    let settings = test_settings_with(&[]);
    archypix_back::routines::Routine::run(
        &archypix_back::routines::job_watchdog::JobWatchdogRoutine::new(db.clone(), settings),
        (),
    )
    .await
    .unwrap();

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::ExtractFailed);
}

/// §6.3: extraction only ever runs on new bytes, so a success is direct evidence against a stale
/// verdict — the old `CASE` that preserved it would strand a file the user replaced with a good one.
#[sqlx::test(migrator = "MIGRATOR")]
async fn a_successful_re_extraction_clears_unsupported_file(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));
    let (pic_id, job_id) = seed_extraction(&db, alice_id, "image/jpeg").await;
    PictureRepository::set_exif_sync_status(&db, pic_id, ExifSyncStatus::UnsupportedFile)
        .await
        .unwrap();

    let claim_token = claim_thumbnail(&app, &token).await;
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &serde_json::json!({
                "claim_token": claim_token,
                "outcome": "done",
                "job": "gen_thumbnail",
                "thumbnails_generated": true,
                "exif": {"extracted": {"gps_lat": 7.5}},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Synced);
    assert_eq!(picture.gps_lat, Some(7.5));
}

/// §6.4: a WebDAV overwrite landing mid-edit sets `extracting`; the edit's completion must not write
/// `synced` over it, nor overwrite the fresh state with its read-back of the pre-overwrite bytes.
#[sqlx::test(migrator = "MIGRATOR")]
async fn an_edit_completion_never_leaves_extracting(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let (claim_token, _config) = claim_edit(&app, &token).await;
    // The overwrite lands while the reconcile is in flight.
    PictureRepository::set_exif_sync_status(&db, pic_id, ExifSyncStatus::Extracting)
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/respond"),
            &token,
            &serde_json::json!({
                "claim_token": claim_token,
                "outcome": "done",
                "job": "edit_picture",
                "thumbnails_generated": false,
                "exif": {"extracted": {"gps_lat": 48.8566, "gps_lng": 2.3522}},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let picture = PictureRepository::find_by_id(&db, pic_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Extracting);
    assert!(
        picture.file_exif.is_none(),
        "the stale read-back must not describe the new bytes"
    );
}
