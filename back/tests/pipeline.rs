//! End-to-end tagging-pipeline tests: live re-derivation, always-on removal, and the
//! service-lifecycle tag handling (promotion on delete, removal on disable).

mod common;

use archypix_back::domain::tag::TagSource;
use archypix_back::domain::tagging::ServiceType;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::infra::routine::pipeline;
use archypix_back::infra::settings::test_settings_with;
use archypix_back::repository::tag::TagRepository;
use archypix_back::repository::tagging::TaggingServiceRepository;
use archypix_back::services;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Run the pipeline once for `user` with throwaway deps + test settings.
async fn run_pipeline(db: &PgPool, user: Uuid) {
    let (fed, cache) = common::make_federation(&test_settings_with(&[]));
    let waker = RoutineHandle::<Uuid>::disconnected();
    pipeline::run_once_for_user(
        db,
        &fed,
        cache.as_ref(),
        &test_settings_with(&[]),
        &waker,
        user,
    )
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

/// Create an empty Rule service.
async fn create_rule_service(db: &PgPool, owner: Uuid) -> Uuid {
    let config = serde_json::json!({ "rules": [] });
    TaggingServiceRepository::create(db, owner, ServiceType::Rule, "", &[], &[], &config)
        .await
        .unwrap()
        .id
}

/// Replace a rule service's whole config with `rules` (mirrors the `PUT /{id}/config` path).
async fn set_rules(db: &PgPool, owner: Uuid, svc: Uuid, rules: Vec<serde_json::Value>) {
    let config = serde_json::json!({ "rules": rules });
    assert!(
        TaggingServiceRepository::set_config(db, owner, svc, ServiceType::Rule, &config)
            .await
            .unwrap()
    );
}

/// Append a rule (predicate + assign_tag) to a rule service and return its generated id.
async fn add_rule(
    db: &PgPool,
    owner: Uuid,
    svc: Uuid,
    predicate: serde_json::Value,
    tag: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let mut rules: Vec<serde_json::Value> = rule_objects(db, owner, svc).await;
    rules.push(serde_json::json!({ "id": id, "predicate": predicate, "assign_tag": tag }));
    set_rules(db, owner, svc, rules).await;
    id
}

/// The raw rule objects of a rule service, in stored order.
async fn rule_objects(db: &PgPool, owner: Uuid, svc: Uuid) -> Vec<serde_json::Value> {
    TaggingServiceRepository::get_by_owner_and_id(db, owner, svc)
        .await
        .unwrap()
        .unwrap()
        .config["rules"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Rule ids of a rule service, in stored order.
async fn rule_ids(db: &PgPool, owner: Uuid, svc: Uuid) -> Vec<Uuid> {
    TaggingServiceRepository::get_by_owner_and_id(db, owner, svc)
        .await
        .unwrap()
        .unwrap()
        .rule_config()
        .unwrap()
        .rules
        .into_iter()
        .map(|r| r.id)
        .collect()
}

/// Create a Rule service with a single "captured in 2024" rule assigning `tag`.
async fn seed_rule_service(db: &PgPool, owner: Uuid, tag: &str) -> Uuid {
    let svc = create_rule_service(db, owner).await;
    add_rule(
        db,
        owner,
        svc,
        serde_json::json!({"field": "captured_at", "year": 2024}),
        tag,
    )
    .await;
    svc
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

    // Drop the rule (empty the config) and re-invalidate — the service now produces nothing.
    set_rules(&db, user, svc, vec![]).await;
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

    let svc = create_rule_service(&db, user).await;
    // (Fujifilm, case-insensitive) AND ISO in [100, 800] AND summer AND exposure ≥ 0.5 s.
    let predicate = serde_json::json!({
        "and": [
            {"field": "camera_brand", "eq": "fujifilm", "ignore_case": true},
            {"field": "iso_speed", "min": 100, "max": 800},
            {"field": "captured_at", "season": "summer"},
            {"field": "exposure_time", "min": 0.5}
        ]
    });
    add_rule(&db, user, svc, predicate, "Camera.Fuji").await;

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

/// The `creator` field (feature 26 integration) resolves to the displayed creator and is matchable
/// end-to-end: a picture with a set creator gets tagged; the owner-default of a NULL-creator picture
/// (which resolves to `@owner:domain`, not the plain string) does not match a plain-text rule.
#[sqlx::test(migrator = "MIGRATOR")]
async fn pipeline_matches_creator_field(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;

    let credited = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO pictures (id, local_user_id, creator) VALUES ($1, $2, 'Grandpa''s camera')"#,
        credited,
        user,
    )
    .execute(&db)
    .await
    .unwrap();
    // A NULL-creator owned picture resolves to the owner identity `@alice:…`, not "Grandpa".
    let owner_default = common::seed_picture(&db, user).await;

    let svc = create_rule_service(&db, user).await;
    add_rule(
        &db,
        user,
        svc,
        serde_json::json!({"field": "creator", "contains": "grandpa", "ignore_case": true}),
        "Family.Grandpa",
    )
    .await;
    // The `owner` field resolves to `@alice:…` for both (owned) pictures — a string match on it.
    add_rule(
        &db,
        user,
        svc,
        serde_json::json!({"field": "owner", "contains": "alice"}),
        "Owned.Alice",
    )
    .await;

    run_pipeline(&db, user).await;

    let tags = TagRepository::list_for_picture(&db, user, credited)
        .await
        .unwrap();
    assert!(
        tags.iter().any(|t| t.tag_path == "Family.Grandpa"),
        "creator rule matched the credited picture"
    );
    assert!(
        tags.iter().any(|t| t.tag_path == "Owned.Alice"),
        "owner rule matched (owned by alice)"
    );
    let other = TagRepository::list_for_picture(&db, user, owner_default)
        .await
        .unwrap();
    assert!(
        !other.iter().any(|t| t.tag_path == "Family.Grandpa"),
        "owner-default creator does not match a plain-text creator rule"
    );
    assert!(
        other.iter().any(|t| t.tag_path == "Owned.Alice"),
        "owner rule matched the owner-default picture too (both owned by alice)"
    );
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
    let rule_id = rule_ids(&db, user, svc).await[0];
    set_rules(
        &db,
        user,
        svc,
        vec![serde_json::json!({
            "id": rule_id,
            "predicate": {"field": "captured_at", "year": 1999},
            "assign_tag": "Photos.Y1999",
        })],
    )
    .await;
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

/// A rule service's config is stored verbatim in array order (order = the submitted order).
#[sqlx::test(migrator = "MIGRATOR")]
async fn rule_config_preserves_array_order(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let svc = create_rule_service(&db, user).await;
    let p = |y: i32| serde_json::json!({"field": "captured_at", "year": y});
    let r1 = add_rule(&db, user, svc, p(2021), "T.A").await;
    let r2 = add_rule(&db, user, svc, p(2022), "T.B").await;
    assert_eq!(
        rule_ids(&db, user, svc).await,
        vec![r1, r2],
        "seeded in creation order"
    );

    // Replacing the config with a reordered array is the only reorder path now.
    let reordered: Vec<_> = rule_objects(&db, user, svc)
        .await
        .into_iter()
        .rev()
        .collect();
    set_rules(&db, user, svc, reordered).await;

    assert_eq!(
        rule_ids(&db, user, svc).await,
        vec![r2, r1],
        "config array order is preserved verbatim"
    );
}

/// A calendar-segmentation service assigns the single resolved band tag (feature 20).
#[sqlx::test(migrator = "MIGRATOR")]
async fn segmentation_assigns_single_band_tag(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_picture_2024(&db, user).await; // captured 2024-06-01

    let config = serde_json::json!({
        "version": 1,
        "root_tag": "Photos.Travel",
        "bands": [
            { "from": "2024-01-01", "to": "2025-01-01", "template": "{year}.{month}",
              "parts": { "month": { "format": { "numeric": false } } } }
        ]
    });
    TaggingServiceRepository::create(&db, user, ServiceType::Segmentation, "", &[], &[], &config)
        .await
        .unwrap();

    run_pipeline(&db, user).await;

    let tags = TagRepository::list_for_picture(&db, user, pic)
        .await
        .unwrap();
    // Only the deepest label is stored (the ancestor is virtual) with source = segment.
    let seg: Vec<_> = tags
        .iter()
        .filter(|t| t.source == TagSource::Segment)
        .collect();
    assert_eq!(seg.len(), 1, "exactly one segment tag");
    assert_eq!(seg[0].tag_path, "Photos.Travel.2024.June");
}
