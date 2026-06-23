//! Feature 14 — Better Batch Editing.
//!
//! Repository- and service-level coverage of the selection descriptor, type-aware aggregation,
//! the deferred-EXIF-job drain, and the batch write surface (tags, EXIF, trash/restore).
//! See `doc/features/14_better_batch_editing.md`.

mod common;

use archypix_back::domain::job::FullExif;
use archypix_back::domain::picture::ExifSyncStatus;
use archypix_back::infra::config::Config;
use archypix_back::infra::exif_drain::ExifDrainWaker;
use archypix_back::infra::pipeline::PipelineWaker;
use archypix_back::repository::picture::{PictureRepository, ResolvedSelection};
use archypix_back::repository::tag::TagRepository;
use archypix_back::services::aggregate::{AggregateRequest, AggregateSection};
use archypix_back::services::jobs::{self, BatchExifMode, ExifBatchOutcome};
use archypix_back::services::selection::{self, FlatFilter, PictureFilter, PictureSelection};
use archypix_back::services::tags::{self, TagBatchOutcome};
use chrono::{NaiveDate, NaiveDateTime};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn dt(y: i32, m: u32, d: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
}

/// Insert an owned picture with the given metadata. `exif` is the camera/lens JSONB.
#[allow(clippy::too_many_arguments)]
async fn seed_owned(
    db: &PgPool,
    user: Uuid,
    mime: &str,
    file_size: i64,
    file_hash: Option<&str>,
    captured_at: Option<NaiveDateTime>,
    gps: Option<(f64, f64)>,
    exif: serde_json::Value,
    thumbnails_done: bool,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO pictures
             (id, local_user_id, mime_type, file_size, file_hash, captured_at,
              gps_lat, gps_lng, exif_data, thumbnails_generated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                   CASE WHEN $10 THEN (now() at time zone 'utc') ELSE NULL END)"#,
        id,
        user,
        mime,
        file_size,
        file_hash,
        captured_at,
        gps.map(|g| g.0),
        gps.map(|g| g.1),
        exif,
        thumbnails_done,
    )
    .execute(db)
    .await
    .unwrap();
    id
}

/// Insert a received picture (owned by a remote user) carrying a remote EXIF snapshot.
async fn seed_received(
    db: &PgPool,
    recipient: Uuid,
    owner: &str,
    remote_exif: serde_json::Value,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query!(
        r#"INSERT INTO pictures
             (id, local_user_id, remote_picture_id, owner_username, owner_instance_domain,
              mime_type, file_size, remote_exif_data)
           VALUES ($1, $2, $3, $4, 'remote.test', 'image/jpeg', 100, $5)"#,
        id,
        recipient,
        Uuid::new_v4().to_string(),
        owner,
        remote_exif,
    )
    .execute(db)
    .await
    .unwrap();
    id
}

fn flat(include_tags: &[&str]) -> PictureSelection {
    PictureSelection {
        query: Some(PictureFilter::Flat(FlatFilter {
            include_tags: include_tags.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        })),
        include_ids: vec![],
        exclude_ids: vec![],
    }
}

// ── Selection resolution ──────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn flat_filter_resolves_and_counts(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = common::seed_picture_with_tag(&db, user, "Photos.Travel").await;
    let _p2 = common::seed_picture_with_tag(&db, user, "Work").await;
    let p3 = common::seed_picture_with_tag(&db, user, "Photos.Family").await;

    let sel = selection::resolve(&db, user, &flat(&["Photos"]))
        .await
        .unwrap();
    let count = PictureRepository::count_selection(&db, user, &sel)
        .await
        .unwrap();
    assert_eq!(count, 2, "two pictures under Photos");

    let ids = PictureRepository::resolve_selection_ids(&db, user, &sel)
        .await
        .unwrap();
    assert!(ids.contains(&p1) && ids.contains(&p3));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn explicit_set_with_exclude(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = common::seed_picture(&db, user).await;
    let p2 = common::seed_picture(&db, user).await;
    let p3 = common::seed_picture(&db, user).await;

    let sel = ResolvedSelection {
        filter: None,
        include_ids: vec![p1, p2, p3],
        exclude_ids: vec![p2],
    };
    let count = PictureRepository::count_selection(&db, user, &sel)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_is_scoped_to_caller(db: PgPool) {
    let alice = common::seed_user(&db, "alice", "pass").await;
    let bob = common::seed_user(&db, "bob", "pass").await;
    let bob_pic = common::seed_picture(&db, bob).await;

    // Alice cannot reach Bob's picture even by explicit id.
    let sel = ResolvedSelection::explicit(vec![bob_pic]);
    let count = PictureRepository::count_selection(&db, alice, &sel)
        .await
        .unwrap();
    assert_eq!(count, 0, "membership is scoped to the caller's holdings");
}

// ── Aggregation ─────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn summary_aggregate(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let o1 = seed_owned(
        &db,
        user,
        "image/jpeg",
        1000,
        Some("h1"),
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let o2 = seed_owned(
        &db,
        user,
        "image/jpeg",
        2000,
        Some("h1"),
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let r1 = seed_received(&db, user, "carol", json!({})).await;

    let sel = ResolvedSelection::explicit(vec![o1, o2, r1]);
    let s = PictureRepository::aggregate_summary(&db, user, &sel)
        .await
        .unwrap();
    assert_eq!(s.count, 3);
    assert_eq!(s.owned_count, 2);
    assert_eq!(s.received_count, 1);
    assert_eq!(s.total_file_size, 3100);
    assert_eq!(s.duplicate_count, 2, "o1 and o2 share file_hash h1");
    assert_eq!(s.owners.len(), 1, "one distinct remote owner");
    assert_eq!(s.owners[0].username, "carol");
    let synced: i64 = s
        .exif_sync
        .iter()
        .find(|(st, _)| *st == ExifSyncStatus::Synced)
        .map(|(_, n)| *n)
        .unwrap_or(0);
    assert_eq!(synced, 3);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn tag_aggregate_ancestor_inclusive_and_manual(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = common::seed_picture_with_tag(&db, user, "Photos.Travel.Alps").await;
    let p2 = common::seed_picture_with_tag(&db, user, "Photos.Work").await;

    let sel = ResolvedSelection::explicit(vec![p1, p2]);
    let aggs = TagRepository::aggregate_tags(&db, user, &sel, false)
        .await
        .unwrap();
    let find = |path: &str| aggs.iter().find(|a| a.path == path);

    // Photos is on both (ancestor-inclusive); the deeper paths on one each.
    assert_eq!(find("Photos").unwrap().count, 2);
    assert_eq!(find("Photos.Travel").unwrap().count, 1);
    assert_eq!(find("Photos.Travel.Alps").unwrap().count, 1);
    // Manual tags drive manual_count.
    assert_eq!(find("Photos").unwrap().manual_count, 2);
    assert_eq!(find("Photos.Travel.Alps").unwrap().manual_count, 1);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn exif_field_aggregates(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = seed_owned(
        &db,
        user,
        "image/jpeg",
        100,
        None,
        Some(dt(2024, 1, 1)),
        Some((45.0, 6.0)),
        json!({"camera_brand": "Fuji", "iso_speed": 200}),
        true,
    )
    .await;
    let p2 = seed_owned(
        &db,
        user,
        "image/jpeg",
        100,
        None,
        Some(dt(2024, 6, 1)),
        Some((47.0, 8.0)),
        json!({"camera_brand": "Fuji", "iso_speed": 800}),
        true,
    )
    .await;

    let sel = ResolvedSelection::explicit(vec![p1, p2]);

    let nums = PictureRepository::aggregate_numeric(
        &db,
        user,
        &sel,
        &[("iso_speed", "(p.exif_data->>'iso_speed')::float8")],
    )
    .await
    .unwrap();
    let iso = &nums[0].1;
    assert_eq!(iso.min, Some(200.0));
    assert_eq!(iso.max, Some(800.0));
    assert_eq!(iso.avg, Some(500.0));

    let gps = PictureRepository::aggregate_gps(&db, user, &sel)
        .await
        .unwrap();
    assert_eq!(gps.lat_min, Some(45.0));
    assert_eq!(gps.lat_max, Some(47.0));
    assert_eq!(gps.centroid_lat, Some(46.0));

    let brand =
        PictureRepository::aggregate_distinct(&db, user, &sel, "(p.exif_data->>'camera_brand')")
            .await
            .unwrap();
    assert_eq!(brand.values, vec![("Fuji".to_string(), 2)]);
    assert_eq!(brand.null_count, 0);

    let dates =
        PictureRepository::aggregate_dates(&db, user, &sel, &[("captured_at", "p.captured_at")])
            .await
            .unwrap();
    let cap = &dates[0].1;
    assert_eq!(cap.min, Some(dt(2024, 1, 1)));
    assert_eq!(cap.max, Some(dt(2024, 6, 1)));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn aggregate_service_summary_and_exif(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = seed_owned(
        &db,
        user,
        "image/jpeg",
        10,
        None,
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let req = AggregateRequest {
        selection: PictureSelection {
            query: None,
            include_ids: vec![p1],
            exclude_ids: vec![],
        },
        sections: Some(vec![AggregateSection::Summary, AggregateSection::Exif]),
        tag_provenance: false,
    };
    let v = archypix_back::services::aggregate::aggregate(&db, user, req)
        .await
        .unwrap();
    assert_eq!(v["count"], json!(1));
    assert!(v.get("exif").is_some());
    assert_eq!(v["exif"]["iso_speed"]["type"], json!("numeric"));
    assert_eq!(v["exif"]["gps"]["type"], json!("gps"));
}

// ── Batch tags ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_tags_apply_and_dry_run(db: PgPool) {
    let waker = PipelineWaker::disconnected();
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = common::seed_picture(&db, user).await;
    let p2 = common::seed_picture(&db, user).await;
    let sel = ResolvedSelection::explicit(vec![p1, p2]);

    // Dry-run reports affected + added without mutating.
    let outcome =
        tags::batch_edit_tags(&db, &waker, user, &sel, &["Holiday".to_string()], &[], true)
            .await
            .unwrap();
    match outcome {
        TagBatchOutcome::DryRun(d) => {
            assert_eq!(d.affected, 2);
            assert_eq!(d.added, Some(2));
        }
        _ => panic!("expected dry-run"),
    }
    assert!(
        TagRepository::list_for_picture(&db, user, p1)
            .await
            .unwrap()
            .is_empty()
    );

    // Apply.
    let outcome = tags::batch_edit_tags(
        &db,
        &waker,
        user,
        &sel,
        &["Holiday".to_string()],
        &[],
        false,
    )
    .await
    .unwrap();
    assert!(matches!(outcome, TagBatchOutcome::Applied { affected: 2 }));
    assert!(
        TagRepository::list_for_picture(&db, user, p1)
            .await
            .unwrap()
            .iter()
            .any(|t| t.tag_path == "Holiday")
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_tags_remove_only_affects_manual(db: PgPool) {
    let waker = PipelineWaker::disconnected();
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = common::seed_picture_with_tag(&db, user, "Trip").await;
    let sel = ResolvedSelection::explicit(vec![p1]);

    let outcome = tags::batch_edit_tags(&db, &waker, user, &sel, &[], &["Trip".to_string()], true)
        .await
        .unwrap();
    match outcome {
        TagBatchOutcome::DryRun(d) => {
            assert_eq!(d.removed, Some(1), "one picture has a manual Trip tag")
        }
        _ => panic!("expected dry-run"),
    }
}

// ── Batch trash / restore ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_trash_then_restore(db: PgPool) {
    use archypix_back::services::pictures::{self, TrashBatchOutcome};
    let waker = PipelineWaker::disconnected();
    let user = common::seed_user(&db, "alice", "pass").await;
    let p1 = common::seed_picture(&db, user).await;
    let p2 = common::seed_picture(&db, user).await;
    let sel = ResolvedSelection::explicit(vec![p1, p2]);

    let out = pictures::batch_set_trashed_selection(&db, &waker, user, &sel, true, false)
        .await
        .unwrap();
    assert!(matches!(out, TrashBatchOutcome::Applied { affected: 2 }));
    let pic = PictureRepository::find_by_id(&db, p1)
        .await
        .unwrap()
        .unwrap();
    assert!(pic.deleted_at.is_some());

    // Restore must include trashed rows (explicit ids bypass the deleted filter).
    let out = pictures::batch_set_trashed_selection(&db, &waker, user, &sel, false, false)
        .await
        .unwrap();
    assert!(matches!(out, TrashBatchOutcome::Applied { affected: 2 }));
    let pic = PictureRepository::find_by_id(&db, p1)
        .await
        .unwrap()
        .unwrap();
    assert!(pic.deleted_at.is_none());
}

// ── Deferred EXIF jobs ────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn owned_batch_exif_defers_then_drain_creates_job(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_owned(
        &db,
        user,
        "image/jpeg",
        100,
        None,
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let sel = ResolvedSelection::explicit(vec![pic]);

    let set = FullExif {
        gps_lat: Some(48.0),
        gps_lng: Some(2.0),
        ..Default::default()
    };
    let mimes: Vec<String> = archypix_common::mime::MIME_TYPES_EXIF
        .iter()
        .map(|m| m.to_lowercase())
        .collect();
    let n = PictureRepository::batch_apply_exif_owned_selection(
        &db,
        user,
        &sel,
        &set,
        &[],
        true,
        &mimes,
    )
    .await
    .unwrap();
    assert_eq!(n, 1);

    let pic_row = PictureRepository::find_by_id(&db, pic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pic_row.exif_sync_status, ExifSyncStatus::PendingJobCreation);
    assert_eq!(pic_row.gps_lat, Some(48.0));

    // The drain turns it into a reconcile job and flips it to `pending`.
    let created = jobs::create_deferred_exif_jobs(&db, 10).await.unwrap();
    assert_eq!(created, 1);
    let pic_row = PictureRepository::find_by_id(&db, pic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pic_row.exif_sync_status, ExifSyncStatus::Pending);
    let jobs = archypix_back::repository::job::JobRepository::list_by_picture(&db, pic, user)
        .await
        .unwrap();
    assert!(
        jobs.iter()
            .any(|j| matches!(j.job_type, archypix_back::domain::job::JobType::EditPicture))
    );

    // A second drain pass is a no-op (no rows left in pending_job_creation).
    assert_eq!(jobs::create_deferred_exif_jobs(&db, 10).await.unwrap(), 0);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn unsupported_owned_batch_exif_marks_unsupported(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    let pic = seed_owned(
        &db,
        user,
        "image/gif",
        100,
        None,
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let sel = ResolvedSelection::explicit(vec![pic]);
    let set = FullExif {
        orientation: Some(3),
        ..Default::default()
    };
    let mimes: Vec<String> = archypix_common::mime::MIME_TYPES_EXIF
        .iter()
        .map(|m| m.to_lowercase())
        .collect();

    let n = PictureRepository::batch_apply_exif_owned_selection(
        &db,
        user,
        &sel,
        &set,
        &[],
        false,
        &mimes,
    )
    .await
    .unwrap();
    assert_eq!(n, 1, "gif is not in the EXIF whitelist");
    let pic_row = PictureRepository::find_by_id(&db, pic)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pic_row.exif_sync_status, ExifSyncStatus::Unsupported);
    assert_eq!(pic_row.orientation, Some(3));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn received_local_override_materialises(db: PgPool) {
    let user = common::seed_user(&db, "alice", "pass").await;
    // Remote owner asserts gps_lat 10.0; recipient overrides to 20.0.
    let pic = seed_received(&db, user, "carol", json!({"gps_lat": 10.0, "gps_lng": 5.0})).await;
    let sel = ResolvedSelection::explicit(vec![pic]);

    let set = FullExif {
        gps_lat: Some(20.0),
        ..Default::default()
    };
    let set_patch = serde_json::to_value(&set).unwrap();
    let n = PictureRepository::batch_apply_exif_received_local_selection(
        &db,
        user,
        &sel,
        &set_patch,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(n, 1);

    let pic_row = PictureRepository::find_by_id(&db, pic)
        .await
        .unwrap()
        .unwrap();
    // Override wins for lat; owner's lng flows through.
    assert_eq!(pic_row.gps_lat, Some(20.0));
    assert_eq!(pic_row.gps_lng, Some(5.0));
    let ov = pic_row.local_exif_overrides.as_ref().unwrap();
    assert_eq!(ov.0.gps_lat, Some(20.0));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn batch_exif_dry_run_partitions(db: PgPool) {
    let cfg = Config::test_defaults();
    let (fed, cache) = common::make_federation(&cfg);
    let waker = PipelineWaker::disconnected();
    let drain = ExifDrainWaker::disconnected();

    let user = common::seed_user(&db, "alice", "pass").await;
    let jpeg = seed_owned(
        &db,
        user,
        "image/jpeg",
        100,
        None,
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let gif = seed_owned(
        &db,
        user,
        "image/gif",
        100,
        None,
        None,
        None,
        json!({}),
        true,
    )
    .await;
    let recv = seed_received(&db, user, "carol", json!({})).await;
    let sel = ResolvedSelection::explicit(vec![jpeg, gif, recv]);

    assert!(!archypix_common::mime::supports_exif("image/gif"));

    let out = jobs::batch_edit_exif_selection(
        &db,
        &waker,
        &drain,
        cache.as_ref(),
        &cfg,
        &fed,
        user,
        "alice",
        &sel,
        FullExif {
            orientation: Some(2),
            ..Default::default()
        },
        vec![],
        BatchExifMode::Local,
        true,
    )
    .await
    .unwrap();
    match out {
        ExifBatchOutcome::DryRun(d) => {
            assert_eq!(d.affected, 3);
            assert_eq!(d.edited, Some(1), "one jpeg supported");
            assert_eq!(d.unsupported, Some(1), "one png unsupported");
            assert_eq!(d.local_override, Some(1), "one received → local override");
            assert_eq!(d.suggested, Some(0));
        }
        _ => panic!("expected dry-run"),
    }
}
