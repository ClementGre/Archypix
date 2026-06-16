mod common;

use archypix_back::infra::config::Config;
use archypix_back::infra::error::AppError;
use archypix_back::repository::picture::{PictureSortField, SortOrder};
use archypix_back::repository::tag::TagRepository;
use archypix_back::services::hierarchy::{self, BrowseParams};
use archypix_back::state::AppState;
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn config() -> Config {
    Config::test_defaults()
}

/// Insert a picture and assign it manual tags.
async fn pic_with_tags(db: &PgPool, user: Uuid, tags: &[&str]) -> Uuid {
    let id = common::seed_picture(db, user).await;
    let owned: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
    if !owned.is_empty() {
        TagRepository::batch_assign(db, user, &[id], &owned)
            .await
            .unwrap();
    }
    id
}

/// Insert a non-manual (e.g. rule) tag directly, bypassing the pipeline.
async fn add_pipeline_tag(db: &PgPool, pic: Uuid, path: &str) {
    sqlx::query(
        "INSERT INTO tags (picture_id, tag_path, source, source_id) \
         VALUES ($1, $2::text::ltree, 'rule'::tag_source, $3)",
    )
    .bind(pic)
    .bind(path)
    .bind(Uuid::new_v4())
    .execute(db)
    .await
    .unwrap();
}

fn browse_params() -> BrowseParams {
    BrowseParams {
        page: 1,
        page_size: 100,
        sort: PictureSortField::default(),
        order: SortOrder::default(),
        include_deleted: false,
        owned_only: false,
        shared_with_me: false,
        captured_after: None,
        captured_before: None,
        thumbnail: None,
    }
}

async fn browse_ids(state: &AppState, user: Uuid, id: Uuid, path: &str) -> HashSet<Uuid> {
    let cfg = config();
    let res = hierarchy::browse(
        &state.db,
        state.cache.as_ref(),
        state.storage.as_ref(),
        &cfg,
        &state.federation,
        user,
        id,
        path,
        browse_params(),
    )
    .await
    .unwrap();
    res.items.into_iter().map(|i| i.id).collect()
}

async fn create(db: &PgPool, user: Uuid, name: &str, config: serde_json::Value) -> Uuid {
    hierarchy::create_hierarchy(db, user, name, &config)
        .await
        .unwrap()
        .id
}

// ─── CRUD & validation ───────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_get_update_delete(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let id = create(&db, user, "Photos", serde_json::json!({"nodes": []})).await;

    let got = hierarchy::get_hierarchy(&db, user, id).await.unwrap();
    assert_eq!(got.name, "Photos");

    hierarchy::update_hierarchy(&db, user, id, Some("Renamed"), Some(false), None)
        .await
        .unwrap();
    let got = hierarchy::get_hierarchy(&db, user, id).await.unwrap();
    assert_eq!(got.name, "Renamed");
    assert!(!got.enabled);

    let list = hierarchy::list_hierarchies(&db, user).await.unwrap();
    assert_eq!(list.len(), 1);

    assert!(hierarchy::delete_hierarchy(&db, user, id).await.unwrap());
    assert!(matches!(
        hierarchy::get_hierarchy(&db, user, id).await,
        Err(AppError::NotFound)
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_rejects_invalid_config(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    // Duplicate sibling names.
    let cfg = serde_json::json!({"nodes": [
        {"id": "a", "kind": "static", "name": "Dup", "children": []},
        {"id": "b", "kind": "static", "name": "Dup", "children": []}
    ]});
    assert!(matches!(
        hierarchy::create_hierarchy(&db, user, "H", &cfg).await,
        Err(AppError::BadRequest(_))
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_rejects_empty_name(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    assert!(matches!(
        hierarchy::create_hierarchy(&db, user, "  ", &serde_json::json!({"nodes": []})).await,
        Err(AppError::BadRequest(_))
    ));
}

// ─── Mirror resolver ───────────────────────────────────────────────────────────

fn mirror_photos(keep_dir: bool) -> serde_json::Value {
    serde_json::json!({"nodes": [
        {"id": "n1", "kind": "mirror", "name": "Photos", "tagRoot": "Photos", "keepDir": keep_dir}
    ]})
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mirror_tree_keep_dir_and_containers(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;

    pic_with_tags(&db, user, &["Photos.Travel.Alps"]).await;
    pic_with_tags(&db, user, &["Photos.Travel"]).await;
    pic_with_tags(&db, user, &["Images.Icons"]).await;

    let id = create(&db, user, "H", mirror_photos(true)).await;

    // Root → Photos only (Images is not under the mirror).
    let root = hierarchy::resolve_tree(&db, user, id, "", 1, false)
        .await
        .unwrap();
    let names: Vec<&str> = root.directories.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, vec!["Photos"]);

    // Photos → Travel (a container dir: no exact tag, but Travel.Alps requires it).
    let photos = hierarchy::resolve_tree(&db, user, id, "Photos", 1, false)
        .await
        .unwrap();
    assert_eq!(
        photos
            .directories
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Travel"]
    );

    // Travel → Alps.
    let travel = hierarchy::resolve_tree(&db, user, id, "Photos/Travel", 1, false)
        .await
        .unwrap();
    assert_eq!(
        travel
            .directories
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alps"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mirror_keep_dir_false_strips_root_label(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    pic_with_tags(&db, user, &["Photos.Travel.Alps"]).await;

    let id = create(&db, user, "H", mirror_photos(false)).await;
    let root = hierarchy::resolve_tree(&db, user, id, "", 1, false)
        .await
        .unwrap();
    // keepDir=false: Photos is stripped, Travel sits at the root level.
    assert_eq!(
        root.directories
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Travel"]
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn root_has_no_direct_files(db: PgPool) {
    // The synthetic root is a pure container: browsing "" surfaces no pictures, even pictures
    // not covered by any node and even for an empty hierarchy (it carries no predicate).
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    pic_with_tags(&db, user, &["Photos.Travel"]).await;
    pic_with_tags(&db, user, &["Images.Icons"]).await; // not under the mirror

    // Empty hierarchy → root must not dump every picture.
    let empty = create(&db, user, "Empty", serde_json::json!({"nodes": []})).await;
    assert!(browse_ids(&state, user, empty, "").await.is_empty());

    // Mirror hierarchy → uncovered pictures (Images.Icons) do not leak into the root listing.
    let id = create(&db, user, "H", mirror_photos(true)).await;
    assert!(browse_ids(&state, user, id, "").await.is_empty());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mirror_browse_most_specific_wins(db: PgPool) {
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let alps = pic_with_tags(&db, user, &["Photos.Travel.Alps"]).await;
    let travel = pic_with_tags(&db, user, &["Photos.Travel"]).await;
    let photos = pic_with_tags(&db, user, &["Photos"]).await;

    let id = create(&db, user, "H", mirror_photos(true)).await;

    // Photos direct files = exact Photos only (Travel/Alps go to deeper dirs).
    assert_eq!(
        browse_ids(&state, user, id, "Photos").await,
        HashSet::from([photos])
    );
    assert_eq!(
        browse_ids(&state, user, id, "Photos/Travel").await,
        HashSet::from([travel])
    );
    assert_eq!(
        browse_ids(&state, user, id, "Photos/Travel/Alps").await,
        HashSet::from([alps])
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mirror_multi_source_deepest_wins(db: PgPool) {
    // A picture with manual Photos.Travel AND rule Photos.Travel.France belongs to France,
    // not directly to Travel (§5.3 governing case).
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let p = pic_with_tags(&db, user, &["Photos.Travel"]).await;
    add_pipeline_tag(&db, p, "Photos.Travel.France").await;

    let id = create(&db, user, "H", mirror_photos(true)).await;

    assert!(
        !browse_ids(&state, user, id, "Photos/Travel")
            .await
            .contains(&p),
        "picture must not be a direct file of Travel"
    );
    assert!(
        browse_ids(&state, user, id, "Photos/Travel/France")
            .await
            .contains(&p),
        "picture appears under France"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mirror_collapsed_rolls_up(db: PgPool) {
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let day1 = pic_with_tags(&db, user, &["Photos.Travel.Alps.Hiking.Day1"]).await;

    let config_json = serde_json::json!({"nodes": [
        {"id": "n1", "kind": "mirror", "name": "Photos", "tagRoot": "Photos", "keepDir": true,
         "collapsed": ["Photos.Travel.Alps.Hiking"]}
    ]});
    let id = create(&db, user, "H", config_json).await;

    // No "Hiking" directory under Alps.
    let alps = hierarchy::resolve_tree(&db, user, id, "Photos/Travel/Alps", 1, false)
        .await
        .unwrap();
    assert!(alps.directories.is_empty(), "Hiking subtree collapsed");

    // The collapsed picture surfaces in the nearest enabled ancestor (Alps).
    assert!(
        browse_ids(&state, user, id, "Photos/Travel/Alps")
            .await
            .contains(&day1)
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn mirror_exclude_prunes(db: PgPool) {
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let only_outdoor = pic_with_tags(&db, user, &["Photos.Outdoor.Trees"]).await;
    let both = pic_with_tags(&db, user, &["Photos.Outdoor.Trees", "Photos.Travel"]).await;

    let config_json = serde_json::json!({"nodes": [
        {"id": "n1", "kind": "mirror", "name": "Photos", "tagRoot": "Photos", "keepDir": true,
         "exclude": ["Photos.Outdoor"]}
    ]});
    let id = create(&db, user, "H", config_json).await;

    // No Outdoor directory.
    let photos = hierarchy::resolve_tree(&db, user, id, "Photos", 1, false)
        .await
        .unwrap();
    assert_eq!(
        photos
            .directories
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Travel"]
    );

    // The picture tagged only under the excluded subtree disappears; the multi-tag one survives
    // under Travel.
    let travel = browse_ids(&state, user, id, "Photos/Travel").await;
    assert!(travel.contains(&both));
    assert!(!travel.contains(&only_outdoor));
}

// ─── Query resolver ─────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn query_nested_inherits_ancestor_predicate(db: PgPool) {
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let both = pic_with_tags(&db, user, &["Starred", "Photos.Travel"]).await;
    let only_travel = pic_with_tags(&db, user, &["Photos.Travel"]).await;
    let only_starred = pic_with_tags(&db, user, &["Starred"]).await;

    // Fav (include Starred) → FavTravel (include Photos.Travel). FavTravel = Starred AND Travel.
    let config_json = serde_json::json!({"nodes": [
        {"id": "fav", "kind": "query", "name": "Fav", "match": "all", "include": ["Starred"],
         "children": [
            {"id": "favtravel", "kind": "query", "name": "FavTravel", "match": "all",
             "include": ["Photos.Travel"]}
         ]}
    ]});
    let id = create(&db, user, "H", config_json).await;

    let fav_travel = browse_ids(&state, user, id, "Fav/FavTravel").await;
    assert_eq!(fav_travel, HashSet::from([both]));

    // Fav direct files = Starred but not in FavTravel child → only_starred (and not only_travel).
    let fav_direct = browse_ids(&state, user, id, "Fav").await;
    assert!(fav_direct.contains(&only_starred));
    assert!(!fav_direct.contains(&both));
    assert!(!fav_direct.contains(&only_travel));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn keep_dir_false_mirror_bubbles_root_tag_into_query_parent(db: PgPool) {
    // A keep_dir=false mirror strips its tagRoot directory, so a picture tagged exactly at
    // tagRoot has no mirror directory of its own. When the mirror sits under a `query` parent,
    // that picture surfaces as a direct file of the query: the query has a predicate, and the
    // mirror never contributes a tagRoot `own_for_parent` term for the parent to subtract.
    // Deeper-tag pictures still go to their own hoisted subdirectory (most-specific-wins).
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let exact = pic_with_tags(&db, user, &["Photos"]).await;

    // Query "Lib" (everything under Photos) containing a keep_dir=false mirror of Photos.
    let config_json = serde_json::json!({"nodes": [
        {"id": "lib", "kind": "query", "name": "Lib", "match": "all", "include": ["Photos"],
         "children": [
            {"id": "m", "kind": "mirror", "tagRoot": "Photos", "keepDir": false}
         ]}
    ]});
    let id = create(&db, user, "H", config_json).await;

    assert_eq!(
        browse_ids(&state, user, id, "Lib").await,
        HashSet::from([exact])
    );

    let deeper = pic_with_tags(&db, user, &["Photos.Travel"]).await;
    // Travel is hoisted to a direct child of Lib; the deeper picture lives there.
    assert_eq!(
        browse_ids(&state, user, id, "Lib/Travel").await,
        HashSet::from([deeper])
    );

    // The exact-`Photos` picture bubbles up into Lib's own direct files; the deeper one does not.
    let lib = browse_ids(&state, user, id, "Lib").await;
    assert!(
        lib.contains(&exact),
        "exact-tagRoot picture surfaces in the query parent"
    );
    assert!(
        !lib.contains(&deeper),
        "deeper picture stays in its own subdirectory"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn query_match_untagged(db: PgPool) {
    let cfg = config();
    let state = common::test_app_state(db.clone(), &cfg);
    let user = common::seed_user(&db, "alice", "pw").await;

    let untagged = common::seed_picture(&db, user).await;
    let tagged = pic_with_tags(&db, user, &["Photos"]).await;

    let config_json = serde_json::json!({"nodes": [
        {"id": "u", "kind": "query", "name": "Untagged", "matchUntagged": true}
    ]});
    let id = create(&db, user, "H", config_json).await;

    let ids = browse_ids(&state, user, id, "Untagged").await;
    assert!(ids.contains(&untagged));
    assert!(!ids.contains(&tagged));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn tree_counts_and_empty_dir_hiding(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    pic_with_tags(&db, user, &["Photos"]).await;

    // One matching query, one empty query.
    let config_json = serde_json::json!({"nodes": [
        {"id": "p", "kind": "query", "name": "HasPhotos", "match": "all", "include": ["Photos"]},
        {"id": "e", "kind": "query", "name": "Empty", "match": "all", "include": ["Nonexistent"]}
    ]});
    let id = create(&db, user, "H", config_json).await;

    // Without counts: both shown, picture_count is null.
    let no_counts = hierarchy::resolve_tree(&db, user, id, "", 1, false)
        .await
        .unwrap();
    assert_eq!(no_counts.directories.len(), 2);
    assert!(
        no_counts
            .directories
            .iter()
            .all(|d| d.picture_count.is_none())
    );

    // With counts: the empty query directory is hidden, the other reports a count.
    let with_counts = hierarchy::resolve_tree(&db, user, id, "", 1, true)
        .await
        .unwrap();
    let names: Vec<&str> = with_counts
        .directories
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(names, vec!["HasPhotos"]);
    assert_eq!(with_counts.directories[0].picture_count, Some(1));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn tree_bad_path_is_not_found(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let id = create(&db, user, "H", mirror_photos(true)).await;
    assert!(matches!(
        hierarchy::resolve_tree(&db, user, id, "DoesNotExist", 1, false).await,
        Err(AppError::NotFound)
    ));
}
