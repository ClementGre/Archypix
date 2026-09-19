use crate::MIGRATOR;
use uuid::Uuid;
use crate::common::{seed_picture, seed_user_bare};
use archypix_back::repository::share_announcement::*;
use archypix_back::repository::share::OutgoingShareRepository;
use sqlx::PgPool;




#[sqlx::test(migrator = "MIGRATOR")]
async fn insert_then_resolve_token(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    let token = ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();
    let resolved = ShareAnnouncementRepository::find_picture_by_token(&db, token)
        .await
        .unwrap();
    assert_eq!(resolved, Some(pic));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn insert_is_idempotent_keeps_token(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    let t1 = ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();
    let t2 = ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();
    assert_eq!(t1, t2, "token stable across re-insert");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_invalidates_token(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    let token = ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();
    ShareAnnouncementRepository::delete(&db, share.id, pic)
        .await
        .unwrap();
    let resolved = ShareAnnouncementRepository::find_picture_by_token(&db, token)
        .await
        .unwrap();
    assert_eq!(resolved, None);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_all_for_share_clears_every_token(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let p1 = seed_picture(&db, owner).await;
    let p2 = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    let t1 = ShareAnnouncementRepository::insert(&db, share.id, p1)
        .await
        .unwrap();
    let t2 = ShareAnnouncementRepository::insert(&db, share.id, p2)
        .await
        .unwrap();
    ShareAnnouncementRepository::delete_all_for_share(&db, share.id)
        .await
        .unwrap();
    assert_eq!(
        ShareAnnouncementRepository::find_picture_by_token(&db, t1)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        ShareAnnouncementRepository::find_picture_by_token(&db, t2)
            .await
            .unwrap(),
        None
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_downstream_for_pictures_returns_recipients(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
        None,
        "carol",
        "carol.com",
        true,
        false,
        true,
        None,
    )
    .await
    .unwrap();
    ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();

    let downstream = ShareAnnouncementRepository::find_downstream_for_pictures(&db, &[pic])
        .await
        .unwrap();
    assert_eq!(downstream.len(), 1);
    assert_eq!(downstream[0].recipient_username, "carol");
    // Owned picture (no remote_picture_id) → announce id falls back to the local id text.
    assert_eq!(downstream[0].announce_id, pic.to_string());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn is_picture_tracked_reflects_membership(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    assert!(
        !ShareAnnouncementRepository::is_picture_tracked(&db, pic)
            .await
            .unwrap(),
        "untracked picture before any announce"
    );
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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
    ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();
    assert!(
        ShareAnnouncementRepository::is_picture_tracked(&db, pic)
            .await
            .unwrap(),
        "tracked after insert"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_stale_announcement_pictures_detects_lag(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    // Announce recording the picture's *current* updated_at → not stale.
    let now: chrono::NaiveDateTime =
        sqlx::query_scalar!("SELECT updated_at FROM pictures WHERE id = $1", pic)
            .fetch_one(&db)
            .await
            .unwrap();
    ShareAnnouncementRepository::insert_with_token(
        &db,
        share.id,
        pic,
        Uuid::new_v4(),
        Some(now),
    )
    .await
    .unwrap();
    assert!(
        ShareAnnouncementRepository::find_stale_announcement_pictures(&db)
            .await
            .unwrap()
            .is_empty(),
        "freshly announced picture is not stale"
    );

    // Bump the picture (the updated_at trigger moves it past announced_updated_at) → stale.
    sqlx::query!("UPDATE pictures SET blurhash = 'abc' WHERE id = $1", pic)
        .execute(&db)
        .await
        .unwrap();
    let stale = ShareAnnouncementRepository::find_stale_announcement_pictures(&db)
        .await
        .unwrap();
    assert_eq!(stale, vec![pic], "picture updated since announce is stale");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn find_stale_announcement_pictures_excludes_dead_shares(db: PgPool) {
    use archypix_back::domain::share::ShareStatus;
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    let announced_at: chrono::NaiveDateTime =
        sqlx::query_scalar!("SELECT updated_at FROM pictures WHERE id = $1", pic)
            .fetch_one(&db)
            .await
            .unwrap();
    ShareAnnouncementRepository::insert_with_token(
        &db,
        share.id,
        pic,
        Uuid::new_v4(),
        Some(announced_at),
    )
    .await
    .unwrap();
    // Make the row lag the picture so it would otherwise be stale.
    sqlx::query!("UPDATE pictures SET blurhash = 'abc' WHERE id = $1", pic)
        .execute(&db)
        .await
        .unwrap();

    // A tombstoned share's lingering row must not resurface as stale (defence-in-depth: the
    // tombstone path also deletes the row).
    OutgoingShareRepository::set_status(&db, share.id, ShareStatus::Tombstoned)
        .await
        .unwrap();
    assert!(
        ShareAnnouncementRepository::find_stale_announcement_pictures(&db)
            .await
            .unwrap()
            .is_empty(),
        "tombstoned share's stale row is excluded"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn update_token_changes_resolution(db: PgPool) {
    let owner = seed_user_bare(&db).await;
    let pic = seed_picture(&db, owner).await;
    let share = OutgoingShareRepository::create(
        &db,
        owner,
        "Photos",
        "Test share",
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

    let old = ShareAnnouncementRepository::insert(&db, share.id, pic)
        .await
        .unwrap();
    let new = Uuid::new_v4();
    ShareAnnouncementRepository::update_token(&db, share.id, pic, new)
        .await
        .unwrap();
    assert_eq!(
        ShareAnnouncementRepository::find_picture_by_token(&db, old)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        ShareAnnouncementRepository::find_picture_by_token(&db, new)
            .await
            .unwrap(),
        Some(pic)
    );
}
