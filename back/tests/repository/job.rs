use crate::MIGRATOR;
use crate::common::seed_user_bare;
use archypix_back::domain::job::Job;
use archypix_back::repository::job::*;
use archypix_back::domain::job::{JobConfig, JobStatus, JobType};
use archypix_common::job::GenThumbnailConfig;
use sqlx::PgPool;
use uuid::Uuid;



async fn seed_job(db: &PgPool, owner_id: Uuid) -> Job {
    let config = JobConfig::GenThumbnail(GenThumbnailConfig {
        picture_id: Uuid::new_v4(),
        is_initial: true,
        metadata_only: false,
    });
    JobRepository::create(db, owner_id, None, &config)
        .await
        .unwrap()
}

// ── claim_next ────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn claim_next_returns_none_when_empty(db: PgPool) {
    let result = JobRepository::claim_next(&db, "worker1", &[JobType::GenThumbnail])
        .await
        .unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn claim_next_marks_job_processing(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let created = seed_job(&db, owner).await;

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .expect("should find a job");

    assert_eq!(claimed.id, created.id);
    assert_eq!(claimed.status, JobStatus::Processing);
    assert!(claimed.claim_token.is_some());
    assert_eq!(claimed.claimed_by.as_deref(), Some("worker1"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn claim_next_respects_job_type_filter(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await; // gen_thumbnail

    // Claim only edit_picture — should find nothing
    let result = JobRepository::claim_next(&db, "worker1", &[JobType::EditPicture])
        .await
        .unwrap();
    assert!(result.is_none());

    // Claim any — should find the gen_thumbnail
    let result = JobRepository::claim_next(&db, "worker1", &[JobType::GenThumbnail])
        .await
        .unwrap();
    assert!(result.is_some());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn claimed_job_is_not_double_claimed(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await;

    JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap();

    // Second claim should find nothing (job is now processing)
    let second = JobRepository::claim_next(&db, "worker2", &[])
        .await
        .unwrap();
    assert!(second.is_none());
}

// ── complete ──────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn complete_with_correct_token_marks_completed(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await;

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();
    let token = claimed.claim_token.unwrap();

    let completed = JobRepository::complete(&db, claimed.id, token, serde_json::json!({}))
        .await
        .unwrap();
    assert!(completed.is_some());
    assert_eq!(completed.unwrap().status, JobStatus::Completed);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn complete_with_wrong_token_returns_none(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await;

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();

    let wrong_token = Uuid::new_v4();
    let result = JobRepository::complete(&db, claimed.id, wrong_token, serde_json::json!({}))
        .await
        .unwrap();
    assert!(result.is_none(), "wrong claim_token must be rejected");

    // Job must still be in processing state
    let job = JobRepository::find_by_id(&db, claimed.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, JobStatus::Processing);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn complete_on_already_completed_job_is_rejected(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await;

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();
    let token = claimed.claim_token.unwrap();

    // First completion
    JobRepository::complete(&db, claimed.id, token, serde_json::json!({}))
        .await
        .unwrap();

    // Second completion with same token — status is no longer 'processing'
    let result = JobRepository::complete(&db, claimed.id, token, serde_json::json!({}))
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── fail ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn fail_with_correct_token_and_retries_resets_to_pending(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await; // default max_retries = 3

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();
    let token = claimed.claim_token.unwrap();

    let failed = JobRepository::fail(&db, claimed.id, token, "transient error", false)
        .await
        .unwrap();
    assert!(failed.is_some());

    let job = JobRepository::find_by_id(&db, claimed.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, JobStatus::Pending, "should reset to pending");
    assert_eq!(job.retry_count, 1);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn fail_permanent_skips_retry(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await;

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();
    let token = claimed.claim_token.unwrap();

    JobRepository::fail(&db, claimed.id, token, "permanent error", true)
        .await
        .unwrap();

    let job = JobRepository::find_by_id(&db, claimed.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, JobStatus::Failed);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn fail_with_wrong_token_is_rejected(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    seed_job(&db, owner).await;

    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();

    let result = JobRepository::fail(&db, claimed.id, Uuid::new_v4(), "error", false)
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── idempotency key ───────────────────────────────────────────────────────

fn thumb_config() -> JobConfig {
    JobConfig::GenThumbnail(GenThumbnailConfig {
        picture_id: Uuid::new_v4(),
        is_initial: true,
        metadata_only: false,
    })
}

/// The point of the mechanism: a retry is a no-op that hands back the job already in flight,
/// not a 409.
#[sqlx::test(migrator = "MIGRATOR")]
async fn duplicate_idempotency_key_returns_the_live_job(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let config = thumb_config();

    let first = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
        .await
        .unwrap();
    assert!(first.created);

    let second = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
        .await
        .unwrap();
    assert!(!second.created, "the retry must not enqueue a second job");
    assert_eq!(second.job.id, first.job.id);
}

/// Liveness scoping: the key guards the queue, not all of history. A terminal job releases it,
/// so re-enqueuing no longer waits on `JobCleanupRoutine`'s retention window.
#[sqlx::test(migrator = "MIGRATOR")]
async fn terminal_job_frees_its_idempotency_key(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let config = thumb_config();

    let first = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
        .await
        .unwrap();
    let claimed = JobRepository::claim_next(&db, "worker1", &[])
        .await
        .unwrap()
        .unwrap();

    // Still live while `processing`.
    let during = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
        .await
        .unwrap();
    assert!(!during.created);
    assert_eq!(during.job.id, first.job.id);

    JobRepository::complete(
        &db,
        claimed.id,
        claimed.claim_token.unwrap(),
        serde_json::json!({}),
    )
    .await
    .unwrap();

    let after = JobRepository::create_idempotent(&db, owner, None, &config, "unique-key")
        .await
        .unwrap();
    assert!(after.created, "a terminal job must release its key");
    assert_ne!(after.job.id, first.job.id);
}

/// Owner scoping comes from the index, so a key no longer has to carry the owner in its text.
#[sqlx::test(migrator = "MIGRATOR")]
async fn idempotency_key_is_scoped_per_owner(db: PgPool) {
    let alice = seed_user_bare(&db).await;
    let bob = seed_user_bare(&db).await;
    let config = thumb_config();

    let a = JobRepository::create_idempotent(&db, alice, None, &config, "shared-key")
        .await
        .unwrap();
    let b = JobRepository::create_idempotent(&db, bob, None, &config, "shared-key")
        .await
        .unwrap();
    assert!(a.created && b.created);
    assert_ne!(a.job.id, b.job.id);
}

/// Unkeyed enqueues are never deduplicated against each other.
#[sqlx::test(migrator = "MIGRATOR")]
async fn create_without_a_key_always_inserts(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let config = thumb_config();

    let first = JobRepository::create(&db, owner, None, &config)
        .await
        .unwrap();
    let second = JobRepository::create(&db, owner, None, &config)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert!(first.idempotency_key.is_none());
}
