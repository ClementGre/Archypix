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
use archypix_back::domain::job::JobStatus;
use archypix_back::infra::config::Config;
use archypix_back::infra::crypto::JwtService;
use archypix_back::repository::job::JobRepository;
use archypix_back::services::jobs::enqueue_thumbnail_job;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── helpers ───────────────────────────────────────────────────────────────────

/// Issue a short-lived worker JWT for `worker_id`.
fn worker_token(config: &Config) -> String {
    let jwt = JwtService::new(&config.worker_jwt_secret, &config.back_domain);
    jwt.issue(
        "test-worker-01",
        None,
        &config.global_domain,
        TokenType::Worker,
        false,
        &config.back_domain,
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
    let config = Config::test_defaults();
    let token = worker_token(&config);

    // Seed: user + picture + pending gen_thumbnail job
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let job = enqueue_thumbnail_job(&db, alice_id, pic_id, true)
        .await
        .unwrap();

    let app =
        archypix_back::api::routes(&config).with_state(common::test_app_state(db.clone(), &config));

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

/// D (re-announce on completion): completing a `gen_thumbnail` job re-marks the picture dirty
/// (`last_pipeline_run_at = NULL`) **only** when it is already in a share's tracking table, so the
/// pipeline re-announces the freshly-hashed metadata. An untracked picture is left alone.
#[sqlx::test(migrator = "MIGRATOR")]
async fn complete_gen_thumbnail_redirties_only_tracked_pictures(db: PgPool) {
    use archypix_back::repository::share::OutgoingShareRepository;
    use archypix_back::repository::share_announcement::ShareAnnouncementRepository;

    let config = Config::test_defaults();
    let token = worker_token(&config);
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let tracked = common::seed_picture(&db, alice_id).await;
    let untracked = common::seed_picture(&db, alice_id).await;

    // Both pictures start "already evaluated" so a re-dirty (NULL) is observable.
    sqlx::query!(
        "UPDATE pictures SET last_pipeline_run_at = now() AT TIME ZONE 'utc' WHERE id = ANY($1)",
        &[tracked, untracked][..],
    )
    .execute(&db)
    .await
    .unwrap();

    // `tracked` is announced under a share → has a tracking row.
    let share = OutgoingShareRepository::create(
        &db,
        alice_id,
        "Photos",
        "S",
        None,
        "bob",
        "other.com",
        true,
        false,
        true,
        None,
    )
    .await
    .unwrap();
    ShareAnnouncementRepository::insert(&db, share.id, tracked)
        .await
        .unwrap();

    let app =
        archypix_back::api::routes(&config).with_state(common::test_app_state(db.clone(), &config));

    for pic in [tracked, untracked] {
        let job = enqueue_thumbnail_job(&db, alice_id, pic, true)
            .await
            .unwrap();
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
        let body = serde_json::json!({
            "claim_token": claim_token,
            "thumbnails_generated": true,
            "file_hash": "hash-for-pic",
            "file_size": 1234
        });
        let r = app
            .clone()
            .oneshot(post_json(
                &format!("/api/worker/jobs/{}/complete", job.id),
                &token,
                &body,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
    }

    let tracked_run: Option<chrono::NaiveDateTime> = sqlx::query_scalar!(
        "SELECT last_pipeline_run_at FROM pictures WHERE id = $1",
        tracked
    )
    .fetch_one(&db)
    .await
    .unwrap();
    let untracked_run: Option<chrono::NaiveDateTime> = sqlx::query_scalar!(
        "SELECT last_pipeline_run_at FROM pictures WHERE id = $1",
        untracked
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(
        tracked_run.is_none(),
        "tracked picture must be re-dirtied for re-announce"
    );
    assert!(
        untracked_run.is_some(),
        "untracked picture must not be re-dirtied"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn worker_fail_permanent_marks_job_failed(db: PgPool) {
    let config = Config::test_defaults();
    let token = worker_token(&config);

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let job = enqueue_thumbnail_job(&db, alice_id, pic_id, true)
        .await
        .unwrap();

    let app =
        archypix_back::api::routes(&config).with_state(common::test_app_state(db.clone(), &config));

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
    let config = Config::test_defaults();
    let token = worker_token(&config);

    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let job = enqueue_thumbnail_job(&db, alice_id, pic_id, false)
        .await
        .unwrap();

    let app =
        archypix_back::api::routes(&config).with_state(common::test_app_state(db.clone(), &config));

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
