//! Tag metadata (feature 34): the enriched tag tree, the partial-upsert write path and the
//! share-boundary seed.

mod common;

use archypix_back::clients::federation::models::SharedTagMeta;
use archypix_back::domain::tag_metadata::{TagMetadata, TagMetadataPatch};
use archypix_back::repository::tag_metadata::TagMetadataRepository;
use archypix_back::services::tag_metadata;
use archypix_common::error::AppError;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn patch(json: serde_json::Value) -> TagMetadataPatch {
    serde_json::from_value(json).unwrap()
}

async fn tag_picture(db: &PgPool, user: Uuid, tag: &str, captured_at: &str) -> Uuid {
    let pic = common::seed_picture_with_tag(db, user, tag).await;
    sqlx::query!(
        "UPDATE pictures SET captured_at = $2::text::timestamp WHERE id = $1",
        pic,
        captured_at,
    )
    .execute(db)
    .await
    .unwrap();
    pic
}

fn item<'a>(
    items: &'a [tag_metadata::TagListItem],
    path: &str,
) -> Option<&'a tag_metadata::TagListItem> {
    items.iter().find(|i| i.path == path)
}

// ── Write path (§2, §4.1) ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn upsert_prunes_a_row_that_ends_up_all_default(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();

    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(
            serde_json::json!({ "tag_path": "Era.2026", "display_name": "Twenty Twenty-Six" }),
        )],
    )
    .await
    .unwrap();
    assert_eq!(
        TagMetadataRepository::list_for_user(&db, user).await.unwrap().len(),
        1
    );

    // Clearing the only field that carried information deletes the row rather than storing a
    // default one (§2).
    let kept = tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({ "tag_path": "Era.2026", "display_name": null }))],
    )
    .await
    .unwrap();
    assert!(kept.is_empty());
    assert!(
        TagMetadataRepository::list_for_user(&db, user).await.unwrap().is_empty(),
        "browsing with default view settings must not litter the table"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn upsert_merges_onto_the_stored_row(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();

    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({
            "tag_path": "Era.2026", "display_name": "Vietnam 🇻🇳", "color": "#a1b2c3"
        }))],
    )
    .await
    .unwrap();
    // A later flush carries only what changed — the display name must survive (§4.1).
    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({ "tag_path": "Era.2026", "sort_index": 300 }))],
    )
    .await
    .unwrap();

    let rows = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_name.as_deref(), Some("Vietnam 🇻🇳"));
    assert_eq!(rows[0].color.as_deref(), Some("#a1b2c3"));
    assert_eq!(rows[0].sort_index, Some(300));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn metadata_is_allowed_on_a_reserved_prefix_and_on_the_root(db: PgPool) {
    // §2: a user may name `SharedToMe.alice_AT_…` "Alice"; §3.3: `""` is the root view.
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();

    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![
            patch(serde_json::json!({
                "tag_path": "SharedToMe.bob_AT_ex_DOT_com", "display_name": "Bob"
            })),
            patch(serde_json::json!({ "tag_path": "", "view_mode": "direct" })),
        ],
    )
    .await
    .unwrap();

    let mut paths: Vec<String> = TagMetadataRepository::list_for_user(&db, user)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.tag_path)
        .collect();
    paths.sort();
    assert_eq!(paths, vec!["", "SharedToMe.bob_AT_ex_DOT_com"]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn upsert_rejects_a_cover_the_caller_does_not_own(db: PgPool) {
    let alice = common::seed_user(&db, "alice", "pw").await;
    let bob = common::seed_user(&db, "bob", "pw").await;
    let bobs_picture = common::seed_picture(&db, bob).await;
    let cache = common::InMemoryCache::new();

    let err = tag_metadata::upsert(
        &db,
        &cache,
        alice,
        vec![patch(serde_json::json!({
            "tag_path": "Era", "cover_picture_id": bobs_picture
        }))],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_drops_the_row_and_keeps_the_tag(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();
    tag_picture(&db, user, "Era.2026", "2026-08-01 09:12:00").await;
    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({ "tag_path": "Era.2026", "display_name": "V" }))],
    )
    .await
    .unwrap();

    tag_metadata::delete(&db, &cache, user, &["Era.2026".to_string()])
        .await
        .unwrap();

    assert!(TagMetadataRepository::list_for_user(&db, user).await.unwrap().is_empty());
    let items = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    assert_eq!(item(&items, "Era.2026").unwrap().live.count, 1, "the tag stays");
}

// ── Read path (§4, §11) ───────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn tree_carries_counts_exact_counts_and_derived_dates(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();
    tag_picture(&db, user, "Era.2026.Vietnam", "2026-08-01 09:12:00").await;
    tag_picture(&db, user, "Era.2026.Vietnam", "2026-08-14 21:40:00").await;
    tag_picture(&db, user, "Era.2026", "2026-01-05 10:00:00").await;

    let items = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();

    let parent = item(&items, "Era.2026").unwrap();
    assert_eq!(parent.live.count, 3, "ancestor-expanded");
    assert_eq!(parent.live.exact_count, 1, "only one picture sits at this path");
    assert_eq!(
        parent.live.date_from.unwrap().to_string(),
        "2026-01-05 10:00:00",
        "the range covers the whole subtree"
    );
    assert_eq!(parent.live.date_to.unwrap().to_string(), "2026-08-14 21:40:00");

    let child = item(&items, "Era.2026.Vietnam").unwrap();
    assert_eq!((child.live.count, child.live.exact_count), (2, 2));
    assert!(parent.trashed.is_none(), "the trashed half is omitted when zero");
    assert!(item(&items, "").is_some(), "the root is always returned");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn trashed_pictures_move_to_the_trashed_half(db: PgPool) {
    // §4/§13.9: a tag whose pictures are all trashed keeps its row and its structure, reported
    // under `trashed` so the client can hide it in the default view.
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();
    let pic = tag_picture(&db, user, "Era.Gone", "2026-03-01 00:00:00").await;
    sqlx::query!(
        "UPDATE pictures SET deleted_at = now() AT TIME ZONE 'utc' WHERE id = $1",
        pic
    )
    .execute(&db)
    .await
    .unwrap();

    let items = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    let tag = item(&items, "Era.Gone").unwrap();
    assert_eq!(tag.live.count, 0);
    let trashed = tag.trashed.as_ref().expect("the trashed half is present");
    assert_eq!((trashed.count, trashed.exact_count), (1, 1));
    assert_eq!(trashed.date_from.unwrap().to_string(), "2026-03-01 00:00:00");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn an_empty_tag_appears_only_when_show_when_empty(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();

    // Annotating without `show_when_empty` leaves no node in the tree (§13.2).
    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({ "tag_path": "Ghost", "display_name": "Ghost" }))],
    )
    .await
    .unwrap();
    let items = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    assert!(item(&items, "Ghost").is_none());

    // A deliberately-created empty tag does appear, and so does its ancestor chain.
    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({
            "tag_path": "Era.2027.Iceland", "show_when_empty": true
        }))],
    )
    .await
    .unwrap();
    let items = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    for path in ["Era", "Era.2027", "Era.2027.Iceland"] {
        let node = item(&items, path)
            .unwrap_or_else(|| panic!("{path} is reachable in the tree"));
        assert_eq!(node.live.count, 0);
    }
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn a_write_busts_the_cached_tree(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();
    tag_picture(&db, user, "Era", "2026-01-01 00:00:00").await;

    let before = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    assert!(item(&before, "Era").unwrap().meta.is_none());

    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({ "tag_path": "Era", "display_name": "Eras" }))],
    )
    .await
    .unwrap();

    let after = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    assert_eq!(
        item(&after, "Era").unwrap().meta.as_ref().unwrap().display_name.as_deref(),
        Some("Eras"),
        "the cached blob was dropped by the write"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn with_sources_adds_the_provenance_breakdown(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();
    tag_picture(&db, user, "Era.2026", "2026-01-01 00:00:00").await;

    let plain = tag_metadata::list_tree(&db, &cache, user, false).await.unwrap();
    assert!(item(&plain, "Era.2026").unwrap().sources.is_none());

    let sourced = tag_metadata::list_tree(&db, &cache, user, true).await.unwrap();
    let sources = item(&sourced, "Era.2026").unwrap().sources.as_ref().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].count, 1);
}

// ── Share boundary (§10.1) ────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn seed_once_never_overwrites_the_recipients_own_row(db: PgPool) {
    let user = common::seed_user(&db, "bob", "pw").await;
    let cache = common::InMemoryCache::new();
    let path = "SharedToMe.alice_AT_ex_DOT_com.Vietnam_2024";
    let announced = SharedTagMeta {
        display_name: Some("Vietnam 🇻🇳".into()),
        description: Some("Two weeks".into()),
        color: Some("#a1b2c3".into()),
        cover_remote_picture_id: None,
    };

    assert!(
        tag_metadata::seed_once(&db, &cache, user, path, &announced)
            .await
            .unwrap()
    );
    // The recipient renames it, then the sender tidies their own label and re-announces.
    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({ "tag_path": path, "display_name": "Alice's trip" }))],
    )
    .await
    .unwrap();
    assert!(
        !tag_metadata::seed_once(&db, &cache, user, path, &announced)
            .await
            .unwrap(),
        "a re-announce must never clobber the recipient"
    );

    let rows = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(rows[0].display_name.as_deref(), Some("Alice's trip"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn seed_resolves_the_cover_through_remote_picture_id(db: PgPool) {
    let user = common::seed_user(&db, "bob", "pw").await;
    let cache = common::InMemoryCache::new();
    let owners_picture_id = Uuid::new_v4();
    let local = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO pictures (id, local_user_id, remote_picture_id, exif_sync_status)
         VALUES ($1, $2, $3, 'synced')",
        local,
        user,
        owners_picture_id.to_string(),
    )
    .execute(&db)
    .await
    .unwrap();

    tag_metadata::seed_once(
        &db,
        &cache,
        user,
        "SharedToMe.alice_AT_ex_DOT_com.V",
        &SharedTagMeta {
            cover_remote_picture_id: Some(owners_picture_id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let rows = TagMetadataRepository::list_for_user(&db, user).await.unwrap();
    assert_eq!(rows[0].cover_picture_id, Some(local));

    // A cover that is not (or not yet) in the share is dropped — the row would be all-default, so
    // nothing is stored and the frontend's first-loaded-photo fallback applies.
    tag_metadata::seed_once(
        &db,
        &cache,
        user,
        "SharedToMe.alice_AT_ex_DOT_com.W",
        &SharedTagMeta {
            cover_remote_picture_id: Some(Uuid::new_v4()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        TagMetadataRepository::find_many(&db, user, &["SharedToMe.alice_AT_ex_DOT_com.W".into()])
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn shared_meta_for_reads_only_the_travelling_fields(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let cache = common::InMemoryCache::new();
    tag_metadata::upsert(
        &db,
        &cache,
        user,
        vec![patch(serde_json::json!({
            "tag_path": "Era", "display_name": "Eras", "webdav_dir_name": "My Eras",
            "sort_index": 100
        }))],
    )
    .await
    .unwrap();

    let meta = tag_metadata::shared_meta_for(&db, user, "Era")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.display_name.as_deref(), Some("Eras"));
    // `webdav_dir_name` and ordering are the recipient's own business (§10.1) — there is nowhere
    // for them to travel in the payload.
    assert!(meta.description.is_none() && meta.color.is_none());

    // A row carrying only local preferences has nothing to announce.
    let _ = TagMetadataRepository::upsert_many(
        &db,
        user,
        &[TagMetadata {
            sort_index: Some(100),
            ..TagMetadata::new("Other".into())
        }],
    )
    .await;
    assert!(
        tag_metadata::shared_meta_for(&db, user, "Other")
            .await
            .unwrap()
            .is_none()
    );
}
