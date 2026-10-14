//! Sensitive pictures (feature 38): the tag-metadata flag, the derived per-picture `sensitive`, the
//! `sensitive` presence filter through listing and selection, and the display-mode setting.

mod common;

use archypix_back::domain::tag_metadata::TagMetadataPatch;
use archypix_back::domain::user_settings::{SensitiveDisplayMode, UserSettingsPatch};
use archypix_back::repository::picture::{PictureListFilter, PictureRepository, PresenceFilter};
use archypix_back::services::selection::{
    self, FlatFilter, PictureFilter, PictureSelection, ScopeParams,
};
use archypix_back::services::{tag_metadata, user_settings};
use archypix_common::error::AppError;
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

async fn flag(db: &PgPool, user: Uuid, tag: &str, sensitive: bool) -> Result<(), AppError> {
    let cache = common::InMemoryCache::new();
    let patch: TagMetadataPatch =
        serde_json::from_value(serde_json::json!({ "tag_path": tag, "sensitive": sensitive }))
            .unwrap();
    tag_metadata::upsert(db, &cache, user, vec![patch]).await.map(|_| ())
}

async fn listed(db: &PgPool, user: Uuid, sensitive: PresenceFilter) -> HashSet<Uuid> {
    let filter = PictureListFilter {
        page: 1,
        page_size: 200,
        sensitive,
        ..Default::default()
    };
    let (items, _) = PictureRepository::list(db, user, &filter).await.unwrap();
    items.into_iter().map(|p| p.id).collect()
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn a_flag_covers_its_subtree_and_every_tag_source(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let deep = common::seed_picture_with_tag(&db, user, "Photos.Medical.Xray").await;
    let ruled = common::seed_picture_with_tag(&db, user, "Photos.Travel").await;
    common::add_pipeline_tag(&db, ruled, "Photos.Medical").await;
    let plain = common::seed_picture_with_tag(&db, user, "Photos.Travel").await;
    let parent_only = common::seed_picture_with_tag(&db, user, "Photos").await;

    flag(&db, user, "Photos.Medical", true).await.unwrap();

    let got = PictureRepository::sensitive_ids(&db, &[deep, ruled, plain, parent_only])
        .await
        .unwrap();
    assert_eq!(got, HashSet::from([deep, ruled]));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn flags_belong_to_the_user_holding_the_row(db: PgPool) {
    let alice = common::seed_user(&db, "alice", "pw").await;
    let bob = common::seed_user(&db, "bob", "pw").await;
    let alices = common::seed_picture_with_tag(&db, alice, "Medical").await;
    let bobs = common::seed_picture_with_tag(&db, bob, "Medical").await;

    flag(&db, alice, "Medical", true).await.unwrap();

    let got = PictureRepository::sensitive_ids(&db, &[alices, bobs]).await.unwrap();
    assert_eq!(got, HashSet::from([alices]));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn the_root_cannot_be_flagged_and_unflagging_prunes_the_row(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    assert!(matches!(
        flag(&db, user, "", true).await,
        Err(AppError::BadRequest(_))
    ));

    flag(&db, user, "Medical", true).await.unwrap();
    flag(&db, user, "Medical", false).await.unwrap();
    let rows = archypix_back::repository::tag_metadata::TagMetadataRepository::list_for_user(
        &db, user,
    )
    .await
    .unwrap();
    assert!(rows.is_empty());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn the_presence_filter_splits_the_listing(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let veiled = common::seed_picture_with_tag(&db, user, "Medical").await;
    let shown = common::seed_picture_with_tag(&db, user, "Travel").await;
    let untagged = common::seed_picture(&db, user).await;
    flag(&db, user, "Medical", true).await.unwrap();

    assert_eq!(
        listed(&db, user, PresenceFilter::Any).await,
        HashSet::from([veiled, shown, untagged])
    );
    assert_eq!(listed(&db, user, PresenceFilter::Present).await, HashSet::from([veiled]));
    assert_eq!(
        listed(&db, user, PresenceFilter::Missing).await,
        HashSet::from([shown, untagged])
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn a_hidden_select_all_skips_veiled_rows_but_keeps_explicit_ids(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let veiled = common::seed_picture_with_tag(&db, user, "Medical").await;
    // A second veiled row the explicit pick below must not drag in.
    common::seed_picture_with_tag(&db, user, "Medical").await;
    let _shown = common::seed_picture_with_tag(&db, user, "Travel").await;
    flag(&db, user, "Medical", true).await.unwrap();

    let query = PictureFilter::Flat(FlatFilter {
        scope: ScopeParams {
            sensitive: PresenceFilter::Missing,
            ..Default::default()
        },
        ..Default::default()
    });
    let all = PictureSelection {
        query: Some(query.clone()),
        ..Default::default()
    };
    let resolved = selection::resolve(&db, user, &all).await.unwrap();
    assert_eq!(PictureRepository::count_selection(&db, user, &resolved).await.unwrap(), 1);

    // An explicitly picked id is the user's choice — never filtered (§9).
    let picked = PictureSelection {
        query: Some(query),
        include_ids: vec![veiled],
        exclude_ids: vec![],
    };
    let resolved = selection::resolve(&db, user, &picked).await.unwrap();
    assert_eq!(PictureRepository::count_selection(&db, user, &resolved).await.unwrap(), 2);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn the_display_mode_defaults_to_blurred_and_patches(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pw").await;
    let s = user_settings::get(&db, user).await.unwrap();
    assert_eq!(s.sensitive_display_mode, SensitiveDisplayMode::Blurred);

    let s = user_settings::update(
        &db,
        user,
        &UserSettingsPatch {
            sensitive_display_mode: Some(SensitiveDisplayMode::Hidden),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(s.sensitive_display_mode, SensitiveDisplayMode::Hidden);
    assert_eq!(s.trash_retention_days, 30, "an omitted field keeps its value");
}
