mod common;

use archypix_back::domain::tag::TagPath;
use archypix_back::domain::tagging::ServiceType;
use archypix_back::routines::RoutineHandle;
use archypix_back::repository::hierarchy::HierarchyRepository;
use archypix_back::repository::picture::ResolvedSelection;
use archypix_back::repository::share::OutgoingShareRepository;
use archypix_back::repository::tag::TagRepository;
use archypix_back::repository::tagging::TaggingServiceRepository;
use archypix_back::services::tags;
use archypix_back::services::tags::cascade_rename;
use archypix_common::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[sqlx::test(migrator = "MIGRATOR")]
async fn edit_picture_tags_rejects_empty_picture_ids(db: PgPool) {
    let user_id = Uuid::new_v4();
    let pipeline_waker = RoutineHandle::<Uuid>::disconnected(); // dummy waker for the test

    let result = tags::batch_edit_tags(
        &db,
        &common::InMemoryCache::new(),
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
        &common::InMemoryCache::new(),
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
        &common::InMemoryCache::new(),
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
        &common::InMemoryCache::new(),
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
        &common::InMemoryCache::new(),
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

// ── Tag rename cascade (was src/services/tags.rs::rename_tests) ───────────────

use serde_json::json;

/// Unlike `common::seed_picture_with_tag`, this stamps `last_pipeline_run_at` and writes the tag
/// row directly — the rename cascade must see an already-reconciled picture.
async fn seed_tagged_picture(db: &PgPool, user_id: Uuid, manual_tag: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, last_pipeline_run_at, exif_sync_status)
         VALUES ($1, $2, now() AT TIME ZONE 'utc', 'synced')",
        id,
        user_id,
    )
    .execute(db)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO tags (picture_id, tag_path, source) VALUES ($1, $2::text::ltree, 'manual')",
        id,
        manual_tag,
    )
    .execute(db)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn cascade_rename_rewrites_everywhere(db: PgPool) {
    let user = common::seed_user_bare(&db).await;
    let pic = seed_tagged_picture(&db, user, "Photos.Travel.Alps").await;
    // Unrelated tag on another picture must survive untouched.
    let other = seed_tagged_picture(&db, user, "Images.Icons").await;

    let share = OutgoingShareRepository::create(
        &db,
        user,
        "Photos.Travel",
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

    let svc = TaggingServiceRepository::create(
        &db,
        user,
        ServiceType::Rule,
        "R",
        &["Photos.Travel".to_string()],
        &[],
        &json!({"rules": [{"id": Uuid::new_v4(), "predicate": {"and": []}, "assign_tag": "Photos.Travel.Auto"}]}),
    )
        .await
        .unwrap();

    let hier = HierarchyRepository::create(
        &db,
        user,
        "H",
        &json!({"version": 2, "nodes": [
            {"id": "m", "kind": "mirror", "tagRoot": "Photos.Travel", "exclude": ["Photos.Travel.Private"]}
        ]}),
    )
        .await
        .unwrap();

    let old = TagPath::from_ltree("Photos.Travel");
    let new = TagPath::from_ltree("Trips.2024");
    let outcome = cascade_rename(&db, &common::InMemoryCache::new(), user, &old, &new)
        .await
        .unwrap();

    assert_eq!(outcome.tags_renamed, 1);
    assert_eq!(outcome.shares_renamed, 1);
    assert_eq!(outcome.services_changed, 1);
    assert_eq!(outcome.hierarchies_changed, 1);
    assert!(outcome.needs_pipeline_wake());

    // Manual tag renamed, unrelated tag untouched.
    let paths = TagRepository::list_manual_paths(&db, pic).await.unwrap();
    assert_eq!(paths, vec!["Trips.2024.Alps".to_string()]);
    assert_eq!(
        TagRepository::list_manual_paths(&db, other).await.unwrap(),
        vec!["Images.Icons".to_string()]
    );

    // Share tag renamed.
    let share = OutgoingShareRepository::get_by_id(&db, share.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(share.tag_path, "Trips.2024");

    // Service gate + config renamed and invalidated.
    let svc = TaggingServiceRepository::get_by_owner_and_id(&db, user, svc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(svc.requires, vec!["Trips.2024".to_string()]);
    assert_eq!(svc.config["rules"][0]["assign_tag"], "Trips.2024.Auto");

    // Hierarchy config renamed (tagRoot + exclude).
    let hier = HierarchyRepository::get_by_owner_and_id(&db, user, hier.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hier.config["nodes"][0]["tagRoot"], "Trips.2024");
    assert_eq!(hier.config["nodes"][0]["exclude"][0], "Trips.2024.Private");

    // Covered picture marked dirty for re-tag + re-announce.
    let dirty: bool = sqlx::query_scalar!(
        r#"SELECT (last_pipeline_run_at IS NULL) AS "d!" FROM pictures WHERE id = $1"#,
        pic,
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert!(dirty);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn cascade_rename_merges_colliding_manual_tag(db: PgPool) {
    let user = common::seed_user_bare(&db).await;
    let pic = seed_tagged_picture(&db, user, "Photos.Travel").await;
    // Same picture already carries the rename target — the source row must be dropped, not error.
    sqlx::query!(
        "INSERT INTO tags (picture_id, tag_path, source) VALUES ($1, 'Photos.Vacation'::ltree, 'manual')",
        pic,
    )
        .execute(&db)
        .await
        .unwrap();

    let outcome = cascade_rename(
        &db,
        &common::InMemoryCache::new(),
        user,
        &TagPath::from_ltree("Photos.Travel"),
        &TagPath::from_ltree("Photos.Vacation"),
    )
    .await
    .unwrap();
    assert_eq!(outcome.tags_renamed, 0, "collision dropped the source row");

    let paths = TagRepository::list_manual_paths(&db, pic).await.unwrap();
    assert_eq!(paths, vec!["Photos.Vacation".to_string()]);
}
