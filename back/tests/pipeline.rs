//! End-to-end tagging-pipeline tests: live re-derivation, always-on removal, and the
//! service-lifecycle tag handling (promotion on delete, removal on disable).

mod common;

use archypix_back::domain::tag::TagSource;
use archypix_back::domain::tagging::ServiceType;
use archypix_back::infra::config::Config;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::infra::routine::pipeline;
use archypix_back::repository::tag::TagRepository;
use archypix_back::repository::tagging::{RuleTaggingRuleRepository, TaggingServiceRepository};
use archypix_back::services;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Run the pipeline once for `user` with throwaway deps + test config.
async fn run_pipeline(db: &PgPool, user: Uuid) {
    let config = Config::test_defaults();
    let (fed, cache) = common::make_federation(&config);
    let waker = RoutineHandle::<uuid::Uuid>::disconnected();
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), &config, &waker, user)
        .await
        .unwrap();
}

/// Insert a picture captured in 2024 so a `captured_at` year=2024 rule matches it.
async fn seed_picture_2024(db: &PgPool, user_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, captured_at) \
         VALUES ($1, $2, '2024-06-01 12:00:00')",
        id,
        user_id,
    )
    .execute(db)
    .await
    .unwrap();
    id
}

/// Create a Rule service with a single "captured in 2024" rule assigning `tag`.
async fn seed_rule_service(db: &PgPool, owner: Uuid, tag: &str) -> Uuid {
    let svc = TaggingServiceRepository::create(db, owner, ServiceType::Rule, "", &[], &[])
        .await
        .unwrap();
    let predicate = serde_json::json!({"field": "captured_at", "year": 2024});
    RuleTaggingRuleRepository::create(db, svc.id, &predicate, tag)
        .await
        .unwrap();
    svc.id
}

fn has_tag(tags: &[archypix_back::domain::tag::Tag], path: &str) -> bool {
    tags.iter().any(|t| t.tag_path == path)
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn pipeline_assigns_matching_rule_tag(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_picture_2024(&db, user).await;
    seed_rule_service(&db, user, "Photos.Y2024").await;

    run_pipeline(&db, user).await;

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    let tag = tags
        .iter()
        .find(|t| t.tag_path == "Photos.Y2024")
        .expect("rule tag assigned");
    assert_eq!(tag.source, TagSource::Rule);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn pipeline_removes_tag_when_rule_no_longer_produces_it(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_picture_2024(&db, user).await;
    let svc = seed_rule_service(&db, user, "Photos.Y2024").await;

    run_pipeline(&db, user).await;
    assert!(has_tag(
        &TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap(),
        "Photos.Y2024"
    ));

    // Drop the rule and re-invalidate — the service now produces nothing.
    let rules = RuleTaggingRuleRepository::list_for_services(&db, &[svc])
        .await
        .unwrap();
    RuleTaggingRuleRepository::delete(&db, user, svc, rules[0].id)
        .await
        .unwrap();
    TaggingServiceRepository::touch_invalidated(&db, svc)
        .await
        .unwrap();

    run_pipeline(&db, user).await;

    assert!(
        !has_tag(
            &TagRepository::list_for_picture(&db, user, pic)
                .await
                .unwrap(),
            "Photos.Y2024"
        ),
        "stale pipeline tag removed"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn pipeline_leaves_manual_tags_untouched(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_picture_2024(&db, user).await;
    let svc = seed_rule_service(&db, user, "Photos.Y2024").await;
    TagRepository::batch_assign(&db, user, &[pic], &["My.Manual".to_string()])
        .await
        .unwrap();

    run_pipeline(&db, user).await;

    // Disable the service → its tags go, manual survives.
    TaggingServiceRepository::update(&db, user, svc, None, Some(false), None, None)
        .await
        .unwrap();
    TagRepository::remove_service_tags(&db, svc).await.unwrap();

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(
        !has_tag(&tags, "Photos.Y2024"),
        "disabled service tag removed"
    );
    assert!(has_tag(&tags, "My.Manual"), "manual tag kept");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn deleting_service_promotes_its_tags_to_manual(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_picture_2024(&db, user).await;
    let svc = seed_rule_service(&db, user, "Photos.Y2024").await;

    run_pipeline(&db, user).await;

    let deleted = services::tagging::delete_service(&db, user, svc, true)
        .await
        .unwrap();
    assert!(deleted);

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    let tag = tags
        .iter()
        .find(|t| t.tag_path == "Photos.Y2024")
        .expect("promoted tag still present");
    assert_eq!(tag.source, TagSource::Manual);
    assert!(tag.source_id.is_none());
}

/// A structured AND predicate over EXIF/file fields (feature 13) is evaluated end-to-end through
/// the pipeline, reading camera fields from `exif_data` and the derived exposure time.
#[sqlx::test(migrator = "MIGRATOR")]
async fn pipeline_evaluates_composed_exif_predicate(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;

    // A Fujifilm photo at ISO 400, 1/2 s exposure, captured in summer 2024.
    let pic = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO pictures (id, local_user_id, captured_at, mime_type, exif_data)
           VALUES ($1, $2, '2024-07-15 10:00:00', 'image/jpeg',
                   '{"camera_brand": "FUJIFILM", "iso_speed": 400,
                     "exposure_time_num": 1, "exposure_time_den": 2}'::jsonb)"#,
        pic,
        user,
    )
    .execute(&db)
    .await
    .unwrap();

    let svc = TaggingServiceRepository::create(&db, user, ServiceType::Rule, "", &[], &[])
        .await
        .unwrap();
    // (Fujifilm, case-insensitive) AND ISO in [100, 800] AND summer AND exposure ≥ 0.5 s.
    let predicate = serde_json::json!({
        "and": [
            {"field": "camera_brand", "eq": "fujifilm", "ignore_case": true},
            {"field": "iso_speed", "min": 100, "max": 800},
            {"field": "captured_at", "season": "summer"},
            {"field": "exposure_time", "min": 0.5}
        ]
    });
    RuleTaggingRuleRepository::create(&db, svc.id, &predicate, "Camera.Fuji")
        .await
        .unwrap();

    run_pipeline(&db, user).await;

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    let tag = tags
        .iter()
        .find(|t| t.tag_path == "Camera.Fuji")
        .expect("composed predicate matched");
    assert_eq!(tag.source, TagSource::Rule);
}

/// Editing a rule's predicate re-derives its tags on the next pipeline run.
#[sqlx::test(migrator = "MIGRATOR")]
async fn editing_rule_predicate_updates_tags(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_picture_2024(&db, user).await;
    let svc = seed_rule_service(&db, user, "Photos.Y2024").await;

    run_pipeline(&db, user).await;
    assert!(has_tag(
        &TagRepository::list_for_picture(&db, user, pic)
            .await
            .unwrap(),
        "Photos.Y2024"
    ));

    // Edit the rule so it now matches a different year — the old tag goes, no new one appears.
    let rules = RuleTaggingRuleRepository::list_for_services(&db, &[svc])
        .await
        .unwrap();
    let new_pred = serde_json::json!({"field": "captured_at", "year": 1999});
    RuleTaggingRuleRepository::update(&db, user, svc, rules[0].id, &new_pred, "Photos.Y1999")
        .await
        .unwrap()
        .expect("rule updated");
    TaggingServiceRepository::touch_invalidated(&db, svc)
        .await
        .unwrap();

    run_pipeline(&db, user).await;
    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    assert!(!has_tag(&tags, "Photos.Y2024"), "old tag removed");
    assert!(
        !has_tag(&tags, "Photos.Y1999"),
        "1999 rule does not match a 2024 picture"
    );
}

/// Reordering a rule service's rules persists a new `position` order.
#[sqlx::test(migrator = "MIGRATOR")]
async fn reordering_rules_persists_order(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let svc = TaggingServiceRepository::create(&db, user, ServiceType::Rule, "", &[], &[])
        .await
        .unwrap();
    let p = |y: i32| serde_json::json!({"field": "captured_at", "year": y});
    let r1 = RuleTaggingRuleRepository::create(&db, svc.id, &p(2021), "T.A")
        .await
        .unwrap();
    let r2 = RuleTaggingRuleRepository::create(&db, svc.id, &p(2022), "T.B")
        .await
        .unwrap();
    assert!(r1.position < r2.position, "rules seeded in creation order");

    RuleTaggingRuleRepository::reorder(&db, user, svc.id, &[r2.id, r1.id])
        .await
        .unwrap();

    let rules = RuleTaggingRuleRepository::list_for_services(&db, &[svc.id])
        .await
        .unwrap();
    assert_eq!(rules[0].id, r2.id, "reordered: B is now first");
    assert_eq!(rules[1].id, r1.id);
}
