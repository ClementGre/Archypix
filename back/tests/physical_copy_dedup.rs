//! Feature 11 — Physical copy ("rescue") & content dedup.
//!
//! Drives the dedup reconciler, the boomerang guard, and the copy endpoint at the service/repository
//! level. See `doc/features/11_physical_copy_and_dedup.md`.

mod common;

use archypix_back::infra::redis::Cache;
use archypix_back::infra::routine::RoutineHandle;
use archypix_back::infra::routine::pipeline::{self, dedup};
use archypix_back::infra::s3::Storage;
use archypix_back::infra::settings::test_settings_with;
use archypix_back::services::pictures;
use chrono::NaiveDateTime;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── Seeding ────────────────────────────────────────────────────────────────────

/// Insert an **owned** picture with a content hash and an optional `deleted_reason`.
async fn seed_owned(db: &PgPool, user: Uuid, content_hash: &str, reason: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    let deleted_at: Option<NaiveDateTime> = reason.map(|_| chrono::Utc::now().naive_utc());
    sqlx::query(
        r#"INSERT INTO pictures (id, local_user_id, content_hash, file_hash, deleted_at,
                                 deleted_reason, thumbnails_generated_at)
           VALUES ($1, $2, $3, $3, $4, $5::picture_deleted_reason, (now() AT TIME ZONE 'utc'))"#,
    )
    .bind(id)
    .bind(user)
    .bind(content_hash)
    .bind(deleted_at)
    .bind(reason)
    .execute(db)
    .await
    .unwrap();
    id
}

/// Insert a **received** picture (owner identity = `owner@test.com`) with a content hash.
async fn seed_received(db: &PgPool, user: Uuid, content_hash: &str, reason: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    let deleted_at: Option<NaiveDateTime> = reason.map(|_| chrono::Utc::now().naive_utc());
    sqlx::query(
        r#"INSERT INTO pictures (id, local_user_id, remote_picture_id, owner_username,
                                 owner_instance_domain, content_hash, file_hash, deleted_at,
                                 deleted_reason, thumbnails_generated_at)
           VALUES ($1, $2, $6, 'owner', 'other.com', $3, $3, $4,
                   $5::picture_deleted_reason, (now() AT TIME ZONE 'utc'))"#,
    )
    .bind(id)
    .bind(user)
    .bind(content_hash)
    .bind(deleted_at)
    .bind(reason)
    .bind(Uuid::new_v4().to_string())
    .execute(db)
    .await
    .unwrap();
    id
}

/// `(deleted_at, deleted_reason)` of a picture.
async fn state(db: &PgPool, id: Uuid) -> (Option<NaiveDateTime>, Option<String>) {
    sqlx::query_as::<_, (Option<NaiveDateTime>, Option<String>)>(
        "SELECT deleted_at, deleted_reason::text FROM pictures WHERE id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .unwrap()
}

async fn is_live(db: &PgPool, id: Uuid) -> bool {
    state(db, id).await.0.is_none()
}

async fn reason(db: &PgPool, id: Uuid) -> Option<String> {
    state(db, id).await.1
}

/// Run one full pipeline pass (includes the dedup reconcile) for `user`.
async fn run_pipeline(db: &PgPool, user: Uuid) {
    let cfg = test_settings_with(&[]);
    let (fed, cache) = common::make_federation(&cfg);
    let waker = RoutineHandle::<Uuid>::disconnected();
    pipeline::run_once_for_user(db, &fed, cache.as_ref(), &cfg, &waker, user)
        .await
        .unwrap();
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn dedup_keeps_one_survivor(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let a = seed_owned(&db, user, "hashA", None).await;
    let b = seed_owned(&db, user, "hashA", None).await;

    run_pipeline(&db, user).await;

    let live: Vec<Uuid> = [a, b]
        .into_iter()
        .zip([is_live(&db, a).await, is_live(&db, b).await])
        .filter_map(|(id, live)| live.then_some(id))
        .collect();
    assert_eq!(live.len(), 1, "exactly one survivor must remain live");
    // The other was hidden as content_dedupe (not manual/boomerang).
    let hidden = if live[0] == a { b } else { a };
    assert_eq!(reason(&db, hidden).await.as_deref(), Some("content_dedupe"));
    // Determinism: lowest id wins among equal candidates.
    assert_eq!(live[0], a.min(b));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn rescue_on_purge_promotes_sibling(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let survivor = seed_owned(&db, user, "hashA", None).await;
    let hidden = seed_owned(&db, user, "hashA", Some("content_dedupe")).await;

    // The survivor disappears via a *system* event (e.g. purge) — hard-delete the row.
    sqlx::query("DELETE FROM pictures WHERE id = $1")
        .bind(survivor)
        .execute(&db)
        .await
        .unwrap();

    run_pipeline(&db, user).await;

    assert!(
        is_live(&db, hidden).await,
        "the hidden content_dedupe copy must be promoted to live (rescue-on-purge)"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn delete_makes_owned_copy_the_trash_representative(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    // A received original is the (kept) live survivor; the user also holds an owned **local** copy
    // hidden as content_dedupe.
    let received = seed_received(&db, user, "hashA", None).await;
    let owned = seed_owned(&db, user, "hashA", Some("content_dedupe")).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    // The user deletes the content (trashes the survivor — the received one they see).
    pictures::trash_picture(&db, &waker, user, received)
        .await
        .unwrap();

    // Priority is respected at delete time: the **owned/local** copy becomes the single `manual`
    // trash representative (so the trash shows owned-deletion messaging, not "owner's copy
    // untouched"), the received one is hidden as boomerang. Neither is live.
    assert_eq!(
        reason(&db, owned).await.as_deref(),
        Some("manual"),
        "the owned local copy is the trash representative, not the clicked received one"
    );
    assert_eq!(reason(&db, received).await.as_deref(), Some("boomerang"));
    assert!(!is_live(&db, owned).await && !is_live(&db, received).await);

    // Stable: the reconciler picks the same best(), so it does not replace the representative.
    run_pipeline(&db, user).await;
    assert_eq!(reason(&db, owned).await.as_deref(), Some("manual"));
    assert_eq!(reason(&db, received).await.as_deref(), Some("boomerang"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn rejected_content_promotes_representative_on_purge(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let a = seed_owned(&db, user, "hashA", None).await;
    let b = seed_owned(&db, user, "hashA", Some("content_dedupe")).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    // Delete the content → one `manual` representative, the other `boomerang` (neither live).
    pictures::trash_picture(&db, &waker, user, a).await.unwrap();
    let rep = if reason(&db, a).await.as_deref() == Some("manual") {
        a
    } else {
        b
    };
    let other = if rep == a { b } else { a };
    assert_eq!(reason(&db, other).await.as_deref(), Some("boomerang"));

    // The representative purges; a boomerang is promoted to the new representative — still trashed,
    // never live (the rejection holds).
    sqlx::query("DELETE FROM pictures WHERE id = $1")
        .bind(rep)
        .execute(&db)
        .await
        .unwrap();
    run_pipeline(&db, user).await;
    assert!(
        !is_live(&db, other).await,
        "rejected content is never promoted to live"
    );
    assert_eq!(
        reason(&db, other).await.as_deref(),
        Some("manual"),
        "a boomerang becomes the new trash representative once the twin is gone"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn restore_lifts_rejection_and_re_enables_rescue(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    // Received survivor + owned hidden copy, so the representative is deterministic (the owned one).
    let received = seed_received(&db, user, "hashA", None).await;
    let owned = seed_owned(&db, user, "hashA", Some("content_dedupe")).await;
    let waker = RoutineHandle::<Uuid>::disconnected();

    // Delete → the owned copy is the manual representative, the received one boomerang.
    pictures::trash_picture(&db, &waker, user, received)
        .await
        .unwrap();
    assert_eq!(reason(&db, owned).await.as_deref(), Some("manual"));
    assert_eq!(reason(&db, received).await.as_deref(), Some("boomerang"));

    // Restore the representative: rejection lifted, its boomerang sibling → content_dedupe.
    pictures::restore_picture(&db, &waker, user, owned)
        .await
        .unwrap();
    assert!(is_live(&db, owned).await);
    assert_eq!(
        reason(&db, received).await.as_deref(),
        Some("content_dedupe"),
        "restore lifts the rejection: boomerang → content_dedupe"
    );

    // With the rejection lifted, a later disappearance of the restored row rescues the sibling.
    sqlx::query("DELETE FROM pictures WHERE id = $1")
        .bind(owned)
        .execute(&db)
        .await
        .unwrap();
    run_pipeline(&db, user).await;
    assert!(
        is_live(&db, received).await,
        "rescue works again after restore"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn chosen_survivor_is_stable_across_reconciles(db: PgPool) {
    use archypix_back::repository::dedup::DedupRepository;
    let user = common::seed_user(&db, "alice", "pass").await;
    let a = seed_owned(&db, user, "hashA", None).await;
    let b = seed_owned(&db, user, "hashA", None).await;

    // First reconcile collapses to the deterministic survivor (lowest id = a).
    run_pipeline(&db, user).await;
    let first = if is_live(&db, a).await { a } else { b };
    let other = if first == a { b } else { a };

    // The user explicitly keeps the *other* copy.
    DedupRepository::set_survivor(&db, user, other)
        .await
        .unwrap();
    assert!(is_live(&db, other).await);
    assert!(!is_live(&db, first).await);

    // A subsequent reconcile must NOT reshuffle back to the deterministic survivor.
    run_pipeline(&db, user).await;
    assert!(
        is_live(&db, other).await,
        "the chosen survivor stays live (stable reconciler)"
    );
    assert_eq!(reason(&db, first).await.as_deref(), Some("content_dedupe"));
}

/// Manual tag paths of a picture, sorted for stable assertions.
async fn manual_tags(db: &PgPool, pic: Uuid) -> Vec<String> {
    let mut v = archypix_back::repository::tag::TagRepository::list_manual_paths(db, pic)
        .await
        .unwrap();
    v.sort();
    v
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn dedup_collapse_merges_manual_tags_onto_survivor(db: PgPool) {
    use archypix_back::repository::tag::TagRepository;
    let user = common::seed_user(&db, "alice", "pass").await;
    let a = seed_owned(&db, user, "hashA", None).await; // lowest id wins → survivor
    let b = seed_owned(&db, user, "hashA", None).await;
    TagRepository::batch_assign(&db, user, &[a], &["Photos.A".to_string()])
        .await
        .unwrap();
    TagRepository::batch_assign(&db, user, &[b], &["Photos.B".to_string()])
        .await
        .unwrap();

    run_pipeline(&db, user).await;

    let survivor = if is_live(&db, a).await { a } else { b };
    assert_eq!(survivor, a.min(b));
    assert_eq!(
        manual_tags(&db, survivor).await,
        vec!["Photos.A".to_string(), "Photos.B".to_string()],
        "the survivor gains the hidden copy's manual tags (union)"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn keep_copy_replaces_manual_tags_from_old_live(db: PgPool) {
    use archypix_back::infra::routine::RoutineHandle;
    use archypix_back::repository::tag::TagRepository;
    let user = common::seed_user(&db, "alice", "pass").await;
    let a = seed_owned(&db, user, "hashA", None).await;
    let b = seed_owned(&db, user, "hashA", None).await;
    // The eventually-hidden copy `b` carries a tag the user will *not* keep.
    TagRepository::batch_assign(&db, user, &[a], &["Photos.X".to_string()])
        .await
        .unwrap();
    TagRepository::batch_assign(&db, user, &[a], &["Photos.Y".to_string()])
        .await
        .unwrap();
    TagRepository::batch_assign(&db, user, &[b], &["Photos.Z".to_string()])
        .await
        .unwrap();

    // Collapse: `a` (lowest id) is the live survivor and gains the union {X, Y, Z}.
    run_pipeline(&db, user).await;
    let live = if is_live(&db, a).await { a } else { b };
    let hidden = if live == a { b } else { a };
    // The user curates the live copy: remove Y.
    TagRepository::batch_remove(&db, user, &[live], &["Photos.Y".to_string()])
        .await
        .unwrap();

    // Switch the live copy to the previously-hidden one.
    let waker = RoutineHandle::<Uuid>::disconnected();
    pictures::set_picture_survivor(&db, &waker, user, hidden)
        .await
        .unwrap();

    assert!(is_live(&db, hidden).await);
    assert_eq!(
        manual_tags(&db, hidden).await,
        vec!["Photos.X".to_string(), "Photos.Z".to_string()],
        "the new live mirrors the old live's curated set (Y stays removed), not its own stale tags"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn trash_view_hides_dedupe_and_boomerang(db: PgPool) {
    use archypix_back::repository::picture::{
        PictureListFilter, PictureRepository, PictureSortField, SortOrder, TrashFilter,
    };
    let user = common::seed_user(&db, "alice", "pass").await;
    let manual = seed_owned(&db, user, "hashA", Some("manual")).await;
    let _dedupe = seed_owned(&db, user, "hashA", Some("content_dedupe")).await;
    let _boomerang = seed_owned(&db, user, "hashB", Some("boomerang")).await;

    let filter = PictureListFilter {
        page: 1,
        page_size: 50,
        sort: PictureSortField::IngestedAt,
        order: SortOrder::Desc,
        predicate: None,
        owned_only: false,
        shared_with_me: false,
        trash: TrashFilter::Only, // trash view
        captured_after: None,
        captured_before: None,
    };
    let (items, _total) = PictureRepository::list(&db, user, &filter).await.unwrap();
    let ids: Vec<_> = items.iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        vec![manual],
        "trash shows only the manual representative"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn arrival_into_rejected_group_boomerangs(db: PgPool) {
    let user = common::seed_user(&db, "bob", "pass").await;
    // Bob previously *manually* deleted this content.
    seed_owned(&db, user, "hashX", Some("manual")).await;

    // A fresh copy of the same content arrives (created live), then is classified.
    let arrival = seed_received(&db, user, "hashX", None).await;
    dedup::classify_arrival(&db, user, arrival).await.unwrap();

    assert_eq!(
        reason(&db, arrival).await.as_deref(),
        Some("boomerang"),
        "a copy matching manually-deleted content must boomerang at arrival"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn arrival_with_live_survivor_is_not_boomeranged(db: PgPool) {
    let user = common::seed_user(&db, "bob", "pass").await;
    seed_owned(&db, user, "hashY", None).await; // a live survivor exists
    let arrival = seed_received(&db, user, "hashY", None).await;
    dedup::classify_arrival(&db, user, arrival).await.unwrap();
    // Not boomeranged; the reconciler will instead content_dedupe it.
    assert_ne!(reason(&db, arrival).await.as_deref(), Some("boomerang"));

    run_pipeline(&db, user).await;
    assert_eq!(
        reason(&db, arrival).await.as_deref(),
        Some("content_dedupe")
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn copy_creates_distinct_owned_identity_with_provenance_root(db: PgPool) {
    let cfg = test_settings_with(&[]);
    let user = common::seed_user(&db, "alice", "pass").await;

    // Seed a received picture (owner @owner:other.com) + its source bytes in S3.
    let source = seed_received(&db, user, "hashZ", None).await;
    let owner_id = common::seed_user(&db, "owner", "pass").await;
    let _ = owner_id; // owner not local to this backend in this fixture
    let storage = Arc::new(common::MockStorage::new());
    // The source is cross-instance here (owner @other.com), so make_federation can't fetch — instead
    // copy an *owned* picture to exercise the same-backend byte path deterministically.
    let owned_source = seed_owned(&db, user, "hashOwned", None).await;
    storage
        .put_object(
            &cfg.get(archypix_back::infra::settings::keys::S3_BUCKET_PICTURES),
            &archypix_back::infra::s3::picture_key(user, owned_source),
            b"original-bytes".to_vec(),
            Some("image/jpeg"),
        )
        .await
        .unwrap();

    let (fed, cache) = common::make_federation(&cfg);
    let waker = RoutineHandle::<Uuid>::disconnected();
    let storage_dyn: Arc<dyn Storage> = storage.clone();

    let copy = pictures::copy_picture(
        &db,
        cache.as_ref() as &dyn Cache,
        storage_dyn.as_ref(),
        &cfg,
        &fed,
        &waker,
        user,
        "alice",
        owned_source,
    )
    .await
    .unwrap();

    assert_ne!(copy.id, owned_source, "a copy is a new, distinct identity");
    assert!(copy.is_owned(), "the copy is owned by the caller");
    assert_eq!(
        copy.copy_source_picture_id.as_deref(),
        Some(owned_source.to_string().as_str()),
        "provenance points at the original"
    );
    assert_eq!(copy.copy_source_owner_username.as_deref(), Some("alice"));
    assert_eq!(
        copy.copy_source_owner_instance.as_deref(),
        Some(
            cfg.get(archypix_back::infra::settings::keys::GLOBAL_DOMAIN)
                .as_str()
        )
    );
    // Bytes were copied to the caller's new key.
    assert!(
        storage
            .get(
                &cfg.get(archypix_back::infra::settings::keys::S3_BUCKET_PICTURES),
                &archypix_back::infra::s3::picture_key(user, copy.id)
            )
            .is_some(),
        "the copy's bytes must exist under the caller's key"
    );
    // A gen_thumbnail job was enqueued for the copy (computes content_hash/thumbnails).
    let jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM jobs WHERE picture_id = $1 AND job_type = 'gen_thumbnail'",
    )
    .bind(copy.id)
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(jobs, 1);

    // The unused received seed is fine; assert it is still present (sanity).
    assert!(state(&db, source).await.0.is_none());
}

/// Set the `creator` column on a picture directly (fixture helper).
async fn set_creator_col(db: &PgPool, id: Uuid, creator: Option<&str>) {
    sqlx::query("UPDATE pictures SET creator = $2 WHERE id = $1")
        .bind(id)
        .bind(creator)
        .execute(db)
        .await
        .unwrap();
}

async fn creator_col(db: &PgPool, id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT creator FROM pictures WHERE id = $1")
        .bind(id)
        .fetch_one(db)
        .await
        .unwrap()
}

/// Feature 26 §6 — attribution travels with the content: a copy carries the source's creator, and
/// an owner-default (NULL) source stays owner-default (the copier owns the copy).
#[sqlx::test(migrator = "MIGRATOR")]
async fn copy_carries_source_creator(db: PgPool) {
    let cfg = test_settings_with(&[]);
    let user = common::seed_user(&db, "alice", "pass").await;
    let storage = Arc::new(common::MockStorage::new());
    let (fed, cache) = common::make_federation(&cfg);
    let waker = RoutineHandle::<Uuid>::disconnected();
    let storage_dyn: Arc<dyn Storage> = storage.clone();

    let put_bytes = |id: Uuid| {
        let storage = storage.clone();
        let cfg = &cfg;
        async move {
            storage
                .put_object(
                    &cfg.get(archypix_back::infra::settings::keys::S3_BUCKET_PICTURES),
                    &archypix_back::infra::s3::picture_key(user, id),
                    b"bytes".to_vec(),
                    Some("image/jpeg"),
                )
                .await
                .unwrap();
        }
    };

    // A manual credit on an owned source is carried verbatim onto the copy.
    let credited = seed_owned(&db, user, "hashCredit", None).await;
    set_creator_col(&db, credited, Some("Grandpa's camera")).await;
    put_bytes(credited).await;
    let copy = pictures::copy_picture(
        &db,
        cache.as_ref() as &dyn Cache,
        storage_dyn.as_ref(),
        &cfg,
        &fed,
        &waker,
        user,
        "alice",
        credited,
    )
    .await
    .unwrap();
    assert_eq!(copy.creator.as_deref(), Some("Grandpa's camera"));

    // An owner-default (NULL) owned source stays owner-default — the copy is the copier's own.
    let plain = seed_owned(&db, user, "hashPlain", None).await;
    put_bytes(plain).await;
    let copy2 = pictures::copy_picture(
        &db,
        cache.as_ref() as &dyn Cache,
        storage_dyn.as_ref(),
        &cfg,
        &fed,
        &waker,
        user,
        "alice",
        plain,
    )
    .await
    .unwrap();
    assert_eq!(copy2.creator, None);
    assert_eq!(creator_col(&db, copy2.id).await, None);
}
