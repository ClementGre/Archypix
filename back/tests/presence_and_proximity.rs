//! Feature 29 — Query presence filters & proximity sorts.
//!
//! Repository-level coverage of the presence arms (per-field + `missing_any` OR), the
//! mutual-exclusion / required-param validation, the directed bracketing lookup (§5), and the
//! time/geo proximity ordering. See `doc/features/29_query_proximity_and_missing_filter.md`.

mod common;

use archypix_back::repository::picture::{
    PictureListFilter, PictureRepository, PictureSortField, PresenceFilter, ResolvedSelection,
    SortOrder,
};
use archypix_common::error::AppError;
use chrono::{NaiveDate, NaiveDateTime};
use sqlx::PgPool;
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Insert an owned picture with an optional capture date and optional GPS.
async fn seed(
    db: &PgPool,
    user: Uuid,
    captured_at: Option<NaiveDateTime>,
    gps: Option<(f64, f64)>,
) -> Uuid {
    let id = Uuid::new_v4();
    let (lat, lng) = match gps {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO pictures (id, local_user_id, captured_at, gps_lat, gps_lng)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(user)
    .bind(captured_at)
    .bind(lat)
    .bind(lng)
    .execute(db)
    .await
    .unwrap();
    id
}

fn dt(y: i32, m: u32, d: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
}

fn base_filter() -> PictureListFilter {
    PictureListFilter {
        page: 1,
        page_size: 200,
        sort: PictureSortField::IngestedAt,
        order: SortOrder::Desc,
        ..Default::default()
    }
}

async fn list_ids(db: &PgPool, user: Uuid, filter: &PictureListFilter) -> Vec<Uuid> {
    let (items, _) = PictureRepository::list(db, user, filter).await.unwrap();
    items.into_iter().map(|p| p.id).collect()
}

// ── Presence filters (§4) ────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn presence_per_field_arms(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    // both, gps-only, date-only, neither.
    let both = seed(&db, user, Some(dt(2020, 1, 1)), Some((45.0, 6.0))).await;
    let gps_only = seed(&db, user, None, Some((46.0, 7.0))).await;
    let date_only = seed(&db, user, Some(dt(2021, 1, 1)), None).await;
    let neither = seed(&db, user, None, None).await;

    let gps_present = list_ids(
        &db,
        user,
        &PictureListFilter {
            gps: PresenceFilter::Present,
            ..base_filter()
        },
    )
    .await;
    assert_eq!(gps_present.len(), 2);
    assert!(gps_present.contains(&both) && gps_present.contains(&gps_only));

    let gps_missing = list_ids(
        &db,
        user,
        &PictureListFilter {
            gps: PresenceFilter::Missing,
            ..base_filter()
        },
    )
    .await;
    assert_eq!(gps_missing.len(), 2);
    assert!(gps_missing.contains(&date_only) && gps_missing.contains(&neither));

    let date_present = list_ids(
        &db,
        user,
        &PictureListFilter {
            capture_date: PresenceFilter::Present,
            ..base_filter()
        },
    )
    .await;
    assert_eq!(date_present.len(), 2);
    assert!(date_present.contains(&both) && date_present.contains(&date_only));

    let date_missing = list_ids(
        &db,
        user,
        &PictureListFilter {
            capture_date: PresenceFilter::Missing,
            ..base_filter()
        },
    )
    .await;
    assert_eq!(date_missing.len(), 2);
    assert!(date_missing.contains(&gps_only) && date_missing.contains(&neither));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn presence_and_composition(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    let _both = seed(&db, user, Some(dt(2020, 1, 1)), Some((45.0, 6.0))).await;
    // The interpolatable set: missing GPS but dated (§2).
    let interpolatable = seed(&db, user, Some(dt(2021, 1, 1)), None).await;
    let _gps_only = seed(&db, user, None, Some((46.0, 7.0))).await;
    let _neither = seed(&db, user, None, None).await;

    let ids = list_ids(
        &db,
        user,
        &PictureListFilter {
            gps: PresenceFilter::Missing,
            capture_date: PresenceFilter::Present,
            ..base_filter()
        },
    )
    .await;
    assert_eq!(ids, vec![interpolatable]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn missing_any_is_the_or(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    let _both = seed(&db, user, Some(dt(2020, 1, 1)), Some((45.0, 6.0))).await;
    let gps_only = seed(&db, user, None, Some((46.0, 7.0))).await;
    let date_only = seed(&db, user, Some(dt(2021, 1, 1)), None).await;
    let neither = seed(&db, user, None, None).await;

    let ids = list_ids(
        &db,
        user,
        &PictureListFilter {
            missing_any: true,
            ..base_filter()
        },
    )
    .await;
    // Everything with *any* gap: all but `both`.
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&gps_only) && ids.contains(&date_only) && ids.contains(&neither));
}

#[test]
fn validate_rejects_missing_any_with_per_field() {
    let f = PictureListFilter {
        missing_any: true,
        gps: PresenceFilter::Missing,
        ..base_filter()
    };
    assert!(matches!(f.validate(), Err(AppError::BadRequest(_))));

    let f = PictureListFilter {
        missing_any: true,
        capture_date: PresenceFilter::Present,
        ..base_filter()
    };
    assert!(matches!(f.validate(), Err(AppError::BadRequest(_))));

    // missing_any alone, or per-field alone, are fine.
    assert!(
        PictureListFilter {
            missing_any: true,
            ..base_filter()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn validate_rejects_proximity_without_reference() {
    assert!(matches!(
        PictureListFilter {
            sort: PictureSortField::TimeNear,
            ..base_filter()
        }
        .validate(),
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        PictureListFilter {
            sort: PictureSortField::GeoNear,
            near_lat: Some(45.0),
            ..base_filter()
        }
        .validate(),
        Err(AppError::BadRequest(_))
    ));
    assert!(
        PictureListFilter {
            sort: PictureSortField::TimeNear,
            near_time: Some(dt(2020, 1, 1)),
            ..base_filter()
        }
        .validate()
        .is_ok()
    );
}

// ── Directed bracketing lookup (§5) ────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn directed_bracketing_returns_one_row_per_side(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    let before2 = seed(&db, user, Some(dt(2020, 1, 1)), Some((1.0, 1.0))).await;
    let before1 = seed(&db, user, Some(dt(2020, 6, 1)), Some((1.0, 1.0))).await;
    // An undated GPS-bearing row must never be an anchor (gps=present, but no captured_at).
    let _undated = seed(&db, user, None, Some((1.0, 1.0))).await;
    let after1 = seed(&db, user, Some(dt(2020, 8, 1)), Some((1.0, 1.0))).await;
    let after2 = seed(&db, user, Some(dt(2021, 1, 1)), Some((1.0, 1.0))).await;
    let reference = dt(2020, 7, 1);

    // before: captured_before + gps present, captured_at DESC, page_size 1.
    let before = list_ids(
        &db,
        user,
        &PictureListFilter {
            page_size: 1,
            sort: PictureSortField::CapturedAt,
            order: SortOrder::Desc,
            gps: PresenceFilter::Present,
            captured_before: Some(reference),
            ..base_filter()
        },
    )
    .await;
    assert_eq!(before, vec![before1]);
    assert_ne!(before, vec![before2]);

    // after: captured_after + gps present, captured_at ASC, page_size 1.
    let after = list_ids(
        &db,
        user,
        &PictureListFilter {
            page_size: 1,
            sort: PictureSortField::CapturedAt,
            order: SortOrder::Asc,
            gps: PresenceFilter::Present,
            captured_after: Some(reference),
            ..base_filter()
        },
    )
    .await;
    assert_eq!(after, vec![after1]);
    assert_ne!(after, vec![after2]);
}

// ── Proximity sorts (§6) ────────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn time_near_orders_by_abs_delta_excludes_undated(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    let far_before = seed(&db, user, Some(dt(2020, 1, 1)), None).await; // 6 months before
    let near_after = seed(&db, user, Some(dt(2020, 7, 15)), None).await; // ~2 weeks after
    let near_before = seed(&db, user, Some(dt(2020, 6, 20)), None).await; // ~10 days before
    let _undated = seed(&db, user, None, None).await;

    let ids = list_ids(
        &db,
        user,
        &PictureListFilter {
            sort: PictureSortField::TimeNear,
            near_time: Some(dt(2020, 7, 1)),
            ..base_filter()
        },
    )
    .await;
    // Nearest by absolute delta first; undated rows are excluded entirely (§6).
    assert_eq!(ids, vec![near_before, near_after, far_before]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn geo_near_orders_by_distance_excludes_ungeotagged(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    // Reference near (45.0, 6.0).
    let close = seed(&db, user, None, Some((45.05, 6.02))).await;
    let mid = seed(&db, user, None, Some((45.5, 6.4))).await;
    let far = seed(&db, user, None, Some((48.0, 9.0))).await;
    let _none = seed(&db, user, None, None).await;

    let ids = list_ids(
        &db,
        user,
        &PictureListFilter {
            sort: PictureSortField::GeoNear,
            near_lat: Some(45.0),
            near_lng: Some(6.0),
            ..base_filter()
        },
    )
    .await;
    // Ungeotagged rows are excluded entirely (§6).
    assert_eq!(ids, vec![close, mid, far]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn geo_near_handles_antimeridian(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    // Reference just west of the antimeridian at +179.9; the true-nearest is at -179.9 (0.2° away),
    // which an equirectangular metric would rank as ~359.8° away.
    let across = seed(&db, user, None, Some((0.0, -179.9))).await;
    let same_side = seed(&db, user, None, Some((0.0, 179.0))).await; // 0.9° away
    let ids = list_ids(
        &db,
        user,
        &PictureListFilter {
            sort: PictureSortField::GeoNear,
            near_lat: Some(0.0),
            near_lng: Some(179.9),
            ..base_filter()
        },
    )
    .await;
    assert_eq!(ids, vec![across, same_side]);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn time_near_stable_tiebreak(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    // Two rows equidistant (same capture time) — the id tiebreaker makes the order total/stable.
    let a = seed(&db, user, Some(dt(2020, 7, 1)), None).await;
    let b = seed(&db, user, Some(dt(2020, 7, 1)), None).await;
    let filter = PictureListFilter {
        sort: PictureSortField::TimeNear,
        near_time: Some(dt(2020, 7, 1)),
        ..base_filter()
    };
    let first = list_ids(&db, user, &filter).await;
    let second = list_ids(&db, user, &filter).await;
    assert_eq!(first, second);
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(first, expected);
}

// ── Selection threading (§7) ────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_count_with_presence_filter(db: PgPool) {
    let user = common::seed_user(&db, "u", "pass").await;
    seed(&db, user, Some(dt(2020, 1, 1)), Some((45.0, 6.0))).await;
    seed(&db, user, Some(dt(2021, 1, 1)), None).await; // missing gps
    seed(&db, user, None, None).await; // missing gps

    let sel = ResolvedSelection {
        filter: Some(PictureListFilter {
            gps: PresenceFilter::Missing,
            ..base_filter()
        }),
        include_ids: vec![],
        exclude_ids: vec![],
    };
    let count = PictureRepository::count_selection(&db, user, &sel)
        .await
        .unwrap();
    assert_eq!(count, 2);
}
