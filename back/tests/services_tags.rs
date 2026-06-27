mod common;

use archypix_back::infra::error::AppError;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::repository::picture::ResolvedSelection;
use archypix_back::repository::tag::TagRepository;
use archypix_back::services::tags;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_picture_tags_rejects_empty_picture_ids(db: PgPool) {
    let user_id = Uuid::new_v4();
    let pipeline_waker = RoutineHandle::<Uuid>::disconnected(); // dummy waker for the test

    let result = tags::batch_edit_tags(
        &db,
        &pipeline_waker,
        user_id,
        &ResolvedSelection::explicit(vec![]),
        &["vacation".to_string()],
        &[],
        false,
    )
    .await;

    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_picture_tags_rejects_no_add_and_no_remove(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let pipeline_waker = RoutineHandle::<Uuid>::disconnected(); // dummy waker for the test

    let result = tags::batch_edit_tags(
        &db,
        &pipeline_waker,
        alice_id,
        &ResolvedSelection::explicit(vec![pic_id.clone()]),
        &[],
        &[],
        false,
    )
    .await;
    assert!(matches!(result, Err(AppError::BadRequest(_))));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_picture_tags_add_is_applied(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture(&db, alice_id).await;
    let pipeline_waker = RoutineHandle::<Uuid>::disconnected(); // dummy waker for the test

    tags::batch_edit_tags(
        &db,
        &pipeline_waker,
        alice_id,
        &ResolvedSelection::explicit(vec![pic_id.clone()]),
        &["vacation".to_string()],
        &[],
        false,
    )
    .await
    .unwrap();

    let stored = TagRepository::list_for_picture(&db, alice_id, pic_id)
        .await
        .unwrap();
    assert!(
        stored.iter().any(|t| t.tag_path == "vacation"),
        "tag must be present after add"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_picture_tags_remove_is_applied(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture_with_tag(&db, alice_id, "vacation").await;
    let pipeline_waker = RoutineHandle::<Uuid>::disconnected(); // dummy waker for the test

    tags::batch_edit_tags(
        &db,
        &pipeline_waker,
        alice_id,
        &ResolvedSelection::explicit(vec![pic_id.clone()]),
        &[],
        &["vacation".to_string()],
        false,
    )
    .await
    .unwrap();

    let stored = TagRepository::list_for_picture(&db, alice_id, pic_id)
        .await
        .unwrap();
    assert!(
        !stored.iter().any(|t| t.tag_path == "vacation"),
        "tag must be gone after remove"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_picture_tags_add_and_remove_are_atomic(db: PgPool) {
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let pic_id = common::seed_picture_with_tag(&db, alice_id, "old").await;
    let pipeline_waker = RoutineHandle::<Uuid>::disconnected(); // dummy waker for the test

    tags::batch_edit_tags(
        &db,
        &pipeline_waker,
        alice_id,
        &ResolvedSelection::explicit(vec![pic_id.clone()]),
        &["new".to_string()],
        &["old".to_string()],
        false,
    )
    .await
    .unwrap();

    let stored = TagRepository::list_for_picture(&db, alice_id, pic_id)
        .await
        .unwrap();
    let paths: Vec<&str> = stored.iter().map(|t| t.tag_path.as_str()).collect();
    assert!(paths.contains(&"new"), "new tag must be present");
    assert!(!paths.contains(&"old"), "old tag must be removed");
}
