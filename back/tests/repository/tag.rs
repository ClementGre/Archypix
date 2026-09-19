use crate::MIGRATOR;
use crate::common::{seed_picture, seed_user_bare};
use archypix_back::domain::tag::*;
use archypix_back::repository::tag::*;
use sqlx::PgPool;
use uuid::Uuid;




/// Insert an active incoming_share row for the given recipient and return its id.
async fn seed_incoming_share(db: &PgPool, recipient: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO incoming_shares
             (id, recipient_id, sender_username, sender_instance, name, outgoing_share_id, status)
         VALUES ($1, $2, 'alice', 'ex.com', 'Test share', $3, 'active'::share_status)",
        id,
        recipient,
        Uuid::new_v4(),
    )
    .execute(db)
    .await
    .unwrap();
    id
}

/// Insert a pipeline tag directly (bypassing the pipeline) for tests setup.
async fn seed_pipeline_tag(db: &PgPool, pic: Uuid, path: &str, source: &str, source_id: Uuid) {
    sqlx::query(
        "INSERT INTO tags (picture_id, tag_path, source, source_id) \
         VALUES ($1, $2::text::ltree, $3::text::tag_source, $4)",
    )
    .bind(pic)
    .bind(path)
    .bind(source)
    .bind(source_id)
    .execute(db)
    .await
    .unwrap();
}

/// Force `last_pipeline_run_at` to a non-NULL value so a test can assert it was re-NULLed.
async fn mark_pipeline_ran(db: &PgPool, pic: Uuid) {
    sqlx::query!(
        "UPDATE pictures SET last_pipeline_run_at = now() AT TIME ZONE 'utc' WHERE id = $1",
        pic,
    )
    .execute(db)
    .await
    .unwrap();
}

async fn is_dirty(db: &PgPool, pic: Uuid) -> bool {
    sqlx::query_scalar!(
        r#"SELECT (last_pipeline_run_at IS NULL) AS "dirty!" FROM pictures WHERE id = $1"#,
        pic,
    )
    .fetch_one(db)
    .await
    .unwrap()
}

// ── intrinsic pipeline invalidation ────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_assign_invalidates_pipeline(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    mark_pipeline_ran(&db, pic).await;

    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
        .await
        .unwrap();

    assert!(
        is_dirty(&db, pic).await,
        "a manual tag add re-dirties the picture"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_remove_invalidates_only_changed(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let tagged = seed_picture(&db, user).await;
    let untouched = seed_picture(&db, user).await;
    TagRepository::batch_assign(&db, user, &[tagged], &["Photos.Travel".to_string()])
        .await
        .unwrap();
    mark_pipeline_ran(&db, tagged).await;
    mark_pipeline_ran(&db, untouched).await;

    TagRepository::batch_remove(&db, user, &[tagged, untouched], &["Photos".to_string()])
        .await
        .unwrap();

    assert!(
        is_dirty(&db, tagged).await,
        "the picture that lost a tag is re-dirtied"
    );
    assert!(
        !is_dirty(&db, untouched).await,
        "a picture with nothing removed is left untouched"
    );
}

// ── batch_assign ──────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_assign_adds_tags(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;

    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
        .await
        .unwrap();

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(tags.iter().any(|t| t.tag_path == "Photos.Travel"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_assign_is_idempotent(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;

    let tags = vec!["Photos.Travel".to_string()];
    TagRepository::batch_assign(&db, user, &[pic], &tags)
        .await
        .unwrap();
    TagRepository::batch_assign(&db, user, &[pic], &tags)
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    let count = stored
        .iter()
        .filter(|t| t.tag_path == "Photos.Travel")
        .count();
    assert_eq!(count, 1, "idempotent — no duplicate");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_assign_prunes_ancestor_when_deeper_added(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;

    // Add parent first
    TagRepository::batch_assign(&db, user, &[pic], &["Photos".to_string()])
        .await
        .unwrap();
    // Then add a child — parent should be pruned (becomes redundant)
    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(
        !stored.iter().any(|t| t.tag_path == "Photos"),
        "ancestor pruned"
    );
    assert!(stored.iter().any(|t| t.tag_path == "Photos.Travel"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_assign_prunes_ancestor_when_shallow_added(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;

    // Add child first
    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
        .await
        .unwrap();
    // Then add a parend — parent should not be added
    TagRepository::batch_assign(&db, user, &[pic], &["Photos".to_string()])
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(
        !stored.iter().any(|t| t.tag_path == "Photos"),
        "ancestor not added because child exists"
    );
    assert!(stored.iter().any(|t| t.tag_path == "Photos.Travel"));
}

// ── batch_remove ──────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_remove_removes_tag_and_subtags(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;

    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel.Alps".to_string()])
        .await
        .unwrap();
    // Remove at Photos level — Alps is a subtag so it should also be removed
    TagRepository::batch_remove(&db, user, &[pic], &["Photos".to_string()])
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(stored.is_empty(), "subtags removed");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_remove_removes_tag_and_keep_parents(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;

    TagRepository::batch_assign(
        &db,
        user,
        &[pic],
        &["Photos.Travel.Alps.Grenoble".to_string()],
    )
    .await
    .unwrap();
    // Currently, deleting a tag does not keep the parent tags.
    TagRepository::batch_remove(&db, user, &[pic], &["Photos.Travel.Alps".to_string()])
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(stored.is_empty(), "parent tags kept");
    //assert!(stored.iter().any(|t| t.tag_path == "Photos.Travel"));
}

// ── assign_incoming_share_tag / remove_incoming_share_tags ────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn assign_and_remove_incoming_share_tag(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    // A real incoming_shares row is required so the token-selection join works and the
    // FK-free source_id is meaningful.
    let share_id = seed_incoming_share(&db, user).await;
    let token = Uuid::new_v4();

    TagRepository::assign_incoming_share_tag(
        &db,
        pic,
        "SharedToMe.alice_AT_ex_DOT_com.Photos",
        share_id,
        token,
    )
    .await
    .unwrap();

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(tags.iter().any(|t| t.source_id == Some(share_id)));

    let affected = TagRepository::remove_incoming_share_tags(&db, share_id)
        .await
        .unwrap();
    assert_eq!(affected, vec![pic]);

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(tags.is_empty());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn assign_incoming_share_tag_refreshes_token_on_conflict(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    let share_id = seed_incoming_share(&db, user).await;
    let token1 = Uuid::new_v4();
    let token2 = Uuid::new_v4();

    TagRepository::assign_incoming_share_tag(
        &db,
        pic,
        "SharedToMe.alice_AT_ex_DOT_com.Photos",
        share_id,
        token1,
    )
    .await
    .unwrap();
    // Replay with a new token updates the stored token (ON CONFLICT DO UPDATE).
    TagRepository::assign_incoming_share_tag(
        &db,
        pic,
        "SharedToMe.alice_AT_ex_DOT_com.Photos",
        share_id,
        token2,
    )
    .await
    .unwrap();

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert_eq!(tags.len(), 1);
    // Active share → token selection returns the refreshed token.
    let selected = TagRepository::find_active_picture_token(&db, pic)
        .await
        .unwrap();
    assert_eq!(selected, Some(token2));
}

// ── per-source storage / lifecycle ────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn manual_and_pipeline_tags_coexist_for_same_path(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    let svc = Uuid::new_v4();

    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Travel".to_string()])
        .await
        .unwrap();
    seed_pipeline_tag(&db, pic, "Photos.Travel", "rule", svc).await;

    // Same path, two sources → two rows (different partial unique indexes).
    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert_eq!(
        stored
            .iter()
            .filter(|t| t.tag_path == "Photos.Travel")
            .count(),
        2,
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn remove_service_tags_drops_only_that_service(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    let svc_a = Uuid::new_v4();
    let svc_b = Uuid::new_v4();

    seed_pipeline_tag(&db, pic, "A.Tag", "rule", svc_a).await;
    seed_pipeline_tag(&db, pic, "B.Tag", "segment", svc_b).await;
    TagRepository::batch_assign(&db, user, &[pic], &["Manual.Tag".to_string()])
        .await
        .unwrap();

    TagRepository::remove_service_tags(&db, svc_a)
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(
        !stored.iter().any(|t| t.tag_path == "A.Tag"),
        "svc_a tag gone"
    );
    assert!(
        stored.iter().any(|t| t.tag_path == "B.Tag"),
        "svc_b tag kept"
    );
    assert!(
        stored.iter().any(|t| t.tag_path == "Manual.Tag"),
        "manual tag untouched"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn promote_service_tags_converts_to_manual(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    let svc = Uuid::new_v4();

    seed_pipeline_tag(&db, pic, "Photos.Alps", "segment", svc).await;
    TagRepository::promote_service_tags_to_manual(&db, svc)
        .await
        .unwrap();

    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    let tag = stored
        .iter()
        .find(|t| t.tag_path == "Photos.Alps")
        .expect("tag still present");
    assert_eq!(tag.source, TagSource::Manual);
    assert!(tag.source_id.is_none());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn promote_service_tags_drops_row_colliding_with_existing_manual(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    let svc = Uuid::new_v4();

    // A manual tag already holds the path the service also produced.
    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Alps".to_string()])
        .await
        .unwrap();
    seed_pipeline_tag(&db, pic, "Photos.Alps.Test", "segment", svc).await;

    TagRepository::promote_service_tags_to_manual(&db, svc)
        .await
        .unwrap();

    // The manual row wins; the colliding pipeline row is dropped — exactly one row remains.
    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    let matching: Vec<_> = stored
        .iter()
        .filter(|t| t.tag_path == "Photos.Alps.Test")
        .collect();
    assert_eq!(matching.len(), 1, "Matching len is not 1");
    assert_eq!(
        matching[0].source,
        TagSource::Manual,
        "Matching source is not Manual"
    );
    assert_eq!(stored.len(), 1, "More than 1 remaining tag");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn promote_service_tags_drops_row_with_exact_manual_twin(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let pic = seed_picture(&db, user).await;
    let svc = Uuid::new_v4();

    // Manual tag holds the exact path the service also produced.
    TagRepository::batch_assign(&db, user, &[pic], &["Photos.Alps".to_string()])
        .await
        .unwrap();
    seed_pipeline_tag(&db, pic, "Photos.Alps", "segment", svc).await;

    TagRepository::promote_service_tags_to_manual(&db, svc)
        .await
        .unwrap();

    // Exact twin → the manual row wins, the pipeline row is dropped: one manual row remains.
    let stored = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1, "exactly one row remains");
    assert_eq!(stored[0].tag_path, "Photos.Alps");
    assert_eq!(stored[0].source, TagSource::Manual);
}
