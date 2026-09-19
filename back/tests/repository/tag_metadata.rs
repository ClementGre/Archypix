use crate::MIGRATOR;
use crate::common::seed_user_bare;
use archypix_back::domain::tag_metadata::*;
use archypix_back::repository::tag_metadata::*;
use sqlx::PgPool;



fn named(path: &str, name: &str) -> TagMetadata {
    TagMetadata {
        display_name: Some(name.to_string()),
        ..TagMetadata::new(path.to_string())
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn upsert_then_list_roundtrips_every_field(db: PgPool) {
    let user = seed_user_bare(&db).await;
    let row = TagMetadata {
        display_name: Some("Vietnam 🇻🇳".into()),
        description: Some("Two weeks".into()),
        color: Some("#a1b2c3".into()),
        date_from: Some("2026-08-01T09:12:00".parse().unwrap()),
        show_when_empty: true,
        sort_index: Some(300),
        children_order: TagOrder::DateFrom,
        view_mode: TagViewMode::All,
        subtag_placement: Some(TagSubtagPlacement::InSections),
        grouping: serde_json::from_value(serde_json::json!({"captured_at": {"kind": "year"}}))
            .unwrap(),
        webdav_dir_name: Some("Vietnam 2026".into()),
        ..TagMetadata::new("Era.2026.Vietnam".into())
    };
    TagMetadataRepository::upsert_many(&db, user, &[row.clone()])
        .await
        .unwrap();

    let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(stored, vec![row]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn upsert_overwrites_the_whole_row(db: PgPool) {
    let user = seed_user_bare(&db).await;
    TagMetadataRepository::upsert_many(&db, user, &[named("A", "first")])
        .await
        .unwrap();
    TagMetadataRepository::upsert_many(&db, user, &[named("A", "second")])
        .await
        .unwrap();
    let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].display_name.as_deref(), Some("second"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn root_row_is_storable_and_findable(db: PgPool) {
    // §3.3: `tag_path = ''` is an ordinary row reached by exactly the same code.
    let user = seed_user_bare(&db).await;
    let root = TagMetadata {
        view_mode: TagViewMode::Direct,
        ..TagMetadata::new(String::new())
    };
    TagMetadataRepository::upsert_many(&db, user, &[root.clone()])
        .await
        .unwrap();
    let found = TagMetadataRepository::find_many(&db, user, &[String::new()])
        .await
        .unwrap();
    assert_eq!(found, vec![root]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn rename_subtree_swaps_the_prefix(db: PgPool) {
    let user = seed_user_bare(&db).await;
    TagMetadataRepository::upsert_many(
        &db,
        user,
        &[
            named("Era.Travel", "Travel"),
            named("Era.Travel.Alps", "Alps"),
            named("Era.Other", "Other"),
        ],
    )
    .await
    .unwrap();

    TagMetadataRepository::rename_subtree(&db, user, "Era.Travel", "Trips.2024")
        .await
        .unwrap();

    let mut paths: Vec<String> = TagMetadataRepository::list_for_user(&db, user)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.tag_path)
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["Era.Other", "Trips.2024", "Trips.2024.Alps"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn rename_subtree_target_wins_on_collision(db: PgPool) {
    let user = seed_user_bare(&db).await;
    TagMetadataRepository::upsert_many(
        &db,
        user,
        &[named("Era.Travel", "source"), named("Era.Trips", "target")],
    )
    .await
    .unwrap();

    TagMetadataRepository::rename_subtree(&db, user, "Era.Travel", "Era.Trips")
        .await
        .unwrap();

    let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(stored.len(), 1, "the source row was dropped");
    assert_eq!(stored[0].display_name.as_deref(), Some("target"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn rename_subtree_never_moves_the_root_row(db: PgPool) {
    // §12: `'' @> anything`, so an unguarded swap would rewrite the whole table.
    let user = seed_user_bare(&db).await;
    TagMetadataRepository::upsert_many(
        &db,
        user,
        &[named("", "root"), named("Era.Travel", "Travel")],
    )
    .await
    .unwrap();

    TagMetadataRepository::rename_subtree(&db, user, "", "Moved")
        .await
        .unwrap();
    TagMetadataRepository::rename_subtree(&db, user, "Era", "Trips")
        .await
        .unwrap();

    let mut paths: Vec<String> = TagMetadataRepository::list_for_user(&db, user)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.tag_path)
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["", "Trips.Travel"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_many_removes_only_the_named_paths(db: PgPool) {
    let user = seed_user_bare(&db).await;
    TagMetadataRepository::upsert_many(&db, user, &[named("A", "a"), named("A.B", "b")])
        .await
        .unwrap();
    TagMetadataRepository::delete_many(&db, user, &["A".to_string()])
        .await
        .unwrap();
    let stored = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].tag_path, "A.B");
}
