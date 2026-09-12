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
use archypix_back::infra::routine::RoutineHandle;
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
        "thumbnails_generated": true,
        "file_hash": "abc123deadbeef",
        "file_size": 204800,
        "width": 800,
        "height": 600
    });
    let resp3 = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/complete", job.id),
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
            &format!("/api/worker/jobs/{}/complete", job.id),
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
        "error": "unsupported image format",
        "permanent": true
    });
    let resp_fail = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/fail", job.id),
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
        "error": "some error",
        "permanent": false
    });
    let resp = app
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/fail", job.id),
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
        "thumbnails_generated": false,
        "exif": {"gps_lat": 48.856599999, "gps_lng": 2.3522000001},
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/complete"),
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
        "thumbnails_generated": false,
        "exif": {"gps_lat": 48.8566, "gps_lng": 2.3522},
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/complete"),
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
        "error": "upload failed",
        "permanent": true,
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/fail"),
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

#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_unsupported_failure_is_terminal(db: PgPool) {
    let settings = test_settings_with(&[]);
    let token = worker_token(&settings);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let (pic_id, job_id) = seed_exif_edit(&db, alice_id).await;
    let app = archypix_back::api::routes(settings.clone())
        .with_state(common::test_app_state(db.clone(), &settings));

    let (claim_token, _config) = claim_edit(&app, &token).await;
    let fail_body = serde_json::json!({
        "claim_token": claim_token,
        "error": "this file cannot carry EXIF",
        "permanent": true,
        "unsupported": true,
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{job_id}/fail"),
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
    assert_eq!(picture.exif_sync_status, ExifSyncStatus::Unsupported);
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
        "thumbnails_generated": true,
        "exif": {"gps_lat": 1.0, "gps_lng": 2.0, "camera_brand": "Nikon"},
    });
    let resp = app
        .clone()
        .oneshot(post_json(
            &format!("/api/worker/jobs/{}/complete", extraction.id),
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
            "thumbnails_generated": false,
            "file_hash": hash,
            "exif": {"gps_lat": 48.8566, "gps_lng": 2.3522},
        });
        let app = app.clone();
        let token = token.clone();
        async move {
            let resp = app
                .oneshot(post_json(
                    &format!("/api/worker/jobs/{job}/complete"),
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
