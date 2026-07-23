//! Feature 30 — Photos fix tools (backend surface).
//!
//! Covers the two backend additions: the `undated_first` date-fix ordering (§4 — undated rows float
//! to the top with a `filename, id` tiebreak) and the `original_file_created_at` source-file-date
//! column round-tripping through create/read. See `doc/features/30_photos_fix_tools.md`.

mod common;

use archypix_back::repository::picture::{
    PictureListFilter, PictureRepository, PictureSortField, SortOrder,
};
use chrono::{NaiveDate, NaiveDateTime};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn dt(y: i32, m: u32, d: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
}

async fn seed_named(
    db: &PgPool,
    user: Uuid,
    filename: &str,
    captured_at: Option<NaiveDateTime>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO pictures (id, local_user_id, filename, captured_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(user)
    .bind(filename)
    .bind(captured_at)
    .execute(db)
    .await
    .unwrap();
    id
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn undated_first_floats_undated_to_top(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    // Two dated + two undated (distinguished by filename for the tiebreak).
    let d_2020 = seed_named(&db, user, "b_dated", Some(dt(2020, 1, 1))).await;
    let d_2021 = seed_named(&db, user, "a_dated", Some(dt(2021, 1, 1))).await;
    let u_b = seed_named(&db, user, "undated_b", None).await;
    let u_a = seed_named(&db, user, "undated_a", None).await;

    let filter = PictureListFilter {
        page: 1,
        page_size: 200,
        sort: PictureSortField::CapturedAt,
        order: SortOrder::Asc,
        undated_first: true,
        ..Default::default()
    };
    let (items, total) = PictureRepository::list(&db, user, &filter).await.unwrap();
    let ids: Vec<Uuid> = items.into_iter().map(|p| p.id).collect();

    // Undated first (by filename asc), then the dated rows by captured_at asc; the count agrees.
    assert_eq!(total, 4);
    assert_eq!(ids, vec![u_a, u_b, d_2020, d_2021]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn original_file_created_at_round_trips(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    let created = PictureRepository::create(
        &db,
        Uuid::new_v4(),
        user,
        Some("photo.jpg"),
        None,
        None,
        None,
        None,
        None,
        Some(dt(2019, 5, 5)), // captured_at
        Some(dt(2018, 1, 2)), // original_file_created_at (source file date)
    )
    .await
    .unwrap();
    assert_eq!(created.original_file_created_at, Some(dt(2018, 1, 2)));

    let loaded = PictureRepository::find_by_id(&db, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.original_file_created_at, Some(dt(2018, 1, 2)));
    // The source file date is never conflated with the capture date.
    assert_eq!(loaded.captured_at, Some(dt(2019, 5, 5)));
}
