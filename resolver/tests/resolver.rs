//! Resolver integration tests (feature 23 §14). Exercise the fleet-admin control plane against a
//! real Postgres (ephemeral DB per test via `#[sqlx::test]`): heartbeat/reachability, stale-prune,
//! selection strategies + pin-delta + capacity gates, registration modes + invite atomicity,
//! operator credential, the layered settings engine, and the delegation-replay client's
//! unreachable-backend behaviour.

use archypix_common::auth::{JwtService, TokenType};
use archypix_common::error::AppError;
use archypix_common::registration::RegistrationMode;
use archypix_resolver::config::{self, SelectionStrategy, setting_keys as sk};
use archypix_resolver::repository;
use archypix_resolver::services::{operator, registration, selection};
use chrono::{Duration, Utc};
use sqlx::PgPool;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── Helpers ──────────────────────────────────────────────────────────────────────

/// Seed a fully-specified backend row (bypassing the heartbeat), for selection/capacity tests.
#[allow(clippy::too_many_arguments)]
async fn seed_backend(
    db: &PgPool,
    domain: &str,
    reachable: bool,
    accepting: bool,
    user_count: i64,
    picture_count: i64,
    storage_bytes: i64,
    max_users: Option<i64>,
) {
    sqlx::query!(
        "INSERT INTO backends
           (back_domain, use_https, internal_url, reachable, accepting_registrations,
            user_count, picture_count, storage_bytes, max_users)
         VALUES ($1, false, $2, $3, $4, $5, $6, $7, $8)",
        domain,
        format!("http://{domain}"),
        reachable,
        accepting,
        user_count,
        picture_count,
        storage_bytes,
        max_users,
    )
        .execute(db)
        .await
        .unwrap();
}

fn cfg(overrides: &[(&str, &str)]) -> archypix_resolver::config::Config {
    config::test_settings_with(overrides)
}

// ── Heartbeat & reachability (§3.2, §7.3, §8.3) ────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn heartbeat_unknown_backend_returns_false(db: PgPool) {
    let stored = repository::record_heartbeat(
        &db,
        "ghost.example.com",
        "tok",
        Utc::now(),
        1,
        2,
        3,
        true,
        "1.0",
    )
        .await
        .unwrap();
    assert!(
        !stored,
        "a heartbeat for an unregistered backend is ignored"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn heartbeat_stores_metrics_and_marks_reachable(db: PgPool) {
    repository::upsert_backend(&db, "b1.example.com", false, "http://b1")
        .await
        .unwrap();
    let before = repository::get_backend(&db, "b1.example.com")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !before.reachable,
        "a freshly self-registered backend starts unreachable"
    );

    let expires = Utc::now() + Duration::seconds(360);
    let stored = repository::record_heartbeat(
        &db,
        "b1.example.com",
        "deleg-tok",
        expires,
        10,
        20,
        30,
        true,
        "1.2.3",
    )
        .await
        .unwrap();
    assert!(stored);

    let after = repository::get_backend(&db, "b1.example.com")
        .await
        .unwrap()
        .unwrap();
    assert!(after.reachable);
    assert_eq!(after.delegation_token.as_deref(), Some("deleg-tok"));
    assert_eq!(
        (after.user_count, after.picture_count, after.storage_bytes),
        (10, 20, 30)
    );
    assert_eq!(after.version.as_deref(), Some("1.2.3"));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn stale_prune_flips_reachability_only_when_expired(db: PgPool) {
    // Fresh token → stays reachable.
    repository::upsert_backend(&db, "fresh.example.com", false, "http://fresh")
        .await
        .unwrap();
    repository::record_heartbeat(
        &db,
        "fresh.example.com",
        "t",
        Utc::now() + Duration::seconds(360),
        0,
        0,
        0,
        true,
        "1",
    )
        .await
        .unwrap();
    // Expired token → gets pruned.
    repository::upsert_backend(&db, "stale.example.com", false, "http://stale")
        .await
        .unwrap();
    repository::record_heartbeat(
        &db,
        "stale.example.com",
        "t",
        Utc::now() - Duration::seconds(1),
        0,
        0,
        0,
        true,
        "1",
    )
        .await
        .unwrap();

    let pruned = repository::prune_stale(&db).await.unwrap();
    assert_eq!(pruned, 1, "only the expired backend is pruned");
    assert!(
        repository::get_backend(&db, "fresh.example.com")
            .await
            .unwrap()
            .unwrap()
            .reachable
    );
    assert!(
        !repository::get_backend(&db, "stale.example.com")
            .await
            .unwrap()
            .unwrap()
            .reachable
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn fleet_totals_sum_across_backends(db: PgPool) {
    seed_backend(&db, "a.example.com", true, true, 5, 50, 500, None).await;
    seed_backend(&db, "b.example.com", true, true, 3, 30, 300, None).await;
    let (u, p, s) = repository::fleet_totals(&db).await.unwrap();
    assert_eq!((u, p, s), (8, 80, 800));
}

// ── Selection strategies (§7) ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_all_ineligible_returns_503(db: PgPool) {
    seed_backend(&db, "unreach.example.com", false, true, 0, 0, 0, None).await; // unreachable
    seed_backend(&db, "closed.example.com", true, false, 0, 0, 0, None).await; // not accepting
    seed_backend(&db, "full.example.com", true, true, 5, 0, 0, Some(5)).await; // at capacity
    let err = selection::pick_backend(&db, &cfg(&[]), None)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::ServiceUnavailable(_)));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_least_users_picks_min(db: PgPool) {
    seed_backend(&db, "big.example.com", true, true, 100, 0, 0, None).await;
    seed_backend(&db, "small.example.com", true, true, 2, 0, 0, None).await;
    let chosen = selection::pick_backend(&db, &cfg(&[]), None).await.unwrap();
    assert_eq!(chosen.back_domain, "small.example.com");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_least_pictures_and_storage(db: PgPool) {
    seed_backend(&db, "few-pics.example.com", true, true, 99, 10, 9999, None).await;
    seed_backend(&db, "many-pics.example.com", true, true, 1, 100, 1, None).await;

    let by_pics =
        selection::pick_backend(&db, &cfg(&[("SELECTION_STRATEGY", "least_pictures")]), None)
            .await
            .unwrap();
    assert_eq!(by_pics.back_domain, "few-pics.example.com");

    let by_storage =
        selection::pick_backend(&db, &cfg(&[("SELECTION_STRATEGY", "least_storage")]), None)
            .await
            .unwrap();
    assert_eq!(by_storage.back_domain, "many-pics.example.com");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_pin_delta_boundary(db: PgPool) {
    // pinned has 1000 pics, best-other has 900 → Δ = 100.
    seed_backend(&db, "pinned.example.com", true, true, 0, 1000, 0, None).await;
    seed_backend(&db, "best.example.com", true, true, 0, 900, 0, None).await;

    // Δ == importance ⇒ honour the pin.
    let at_boundary = selection::pick_backend(
        &db,
        &cfg(&[
            ("SELECTION_STRATEGY", "least_pictures"),
            ("PIN_IMPORTANCE", "100"),
        ]),
        Some("pinned.example.com"),
    )
        .await
        .unwrap();
    assert_eq!(at_boundary.back_domain, "pinned.example.com");

    // Δ > importance ⇒ pick the metric-best instead.
    let over = selection::pick_backend(
        &db,
        &cfg(&[
            ("SELECTION_STRATEGY", "least_pictures"),
            ("PIN_IMPORTANCE", "99"),
        ]),
        Some("pinned.example.com"),
    )
        .await
        .unwrap();
    assert_eq!(over.back_domain, "best.example.com");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_pin_never_beats_capacity(db: PgPool) {
    // The pinned backend is full → ineligible → never chosen even at max importance.
    seed_backend(&db, "pinned-full.example.com", true, true, 5, 0, 0, Some(5)).await;
    seed_backend(&db, "open.example.com", true, true, 1, 0, 0, None).await;
    let chosen = selection::pick_backend(
        &db,
        &cfg(&[
            ("SELECTION_STRATEGY", "least_users"),
            ("PIN_IMPORTANCE", "1000000"),
        ]),
        Some("pinned-full.example.com"),
    )
        .await
        .unwrap();
    assert_eq!(chosen.back_domain, "open.example.com");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_round_robin_cycles_and_honours_importance(db: PgPool) {
    seed_backend(&db, "rr-a.example.com", true, true, 0, 0, 0, None).await;
    seed_backend(&db, "rr-b.example.com", true, true, 0, 0, 0, None).await;

    // Both never-selected → first pick touches one; second pick must choose the other.
    let c = cfg(&[("SELECTION_STRATEGY", "round_robin")]);
    let first = selection::pick_backend(&db, &c, None)
        .await
        .unwrap()
        .back_domain;
    let second = selection::pick_backend(&db, &c, None)
        .await
        .unwrap()
        .back_domain;
    assert_ne!(
        first, second,
        "round-robin advances to the least-recently-selected backend"
    );

    // importance ≥ 1 ⇒ follow the pin regardless of the cursor.
    let pinned = selection::pick_backend(
        &db,
        &cfg(&[
            ("SELECTION_STRATEGY", "round_robin"),
            ("PIN_IMPORTANCE", "1"),
        ]),
        Some("rr-a.example.com"),
    )
        .await
        .unwrap();
    assert_eq!(pinned.back_domain, "rr-a.example.com");
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn selection_static_uses_configured_backend(db: PgPool) {
    seed_backend(&db, "s1.example.com", true, true, 1, 0, 0, None).await;
    seed_backend(&db, "s2.example.com", true, true, 100, 0, 0, None).await;
    let chosen = selection::pick_backend(
        &db,
        &cfg(&[
            ("SELECTION_STRATEGY", "static"),
            ("STATIC_BACKEND", "s2.example.com"),
        ]),
        None,
    )
        .await
        .unwrap();
    assert_eq!(
        chosen.back_domain, "s2.example.com",
        "static ignores metrics, uses the pinned backend"
    );
}

// ── Registration modes + invite atomicity (§6) ─────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn registration_open_ignores_absent_and_invalid_code(db: PgPool) {
    let c = cfg(&[]); // registration_mode defaults to open
    let auth = registration::authorize(&db, &c, None).await.unwrap();
    assert!(auth.invite.is_none());
    // Unknown code in Open mode is ignored (not an error).
    let auth = registration::authorize(&db, &c, Some("nope"))
        .await
        .unwrap();
    assert!(auth.invite.is_none());
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn registration_invite_mode_requires_valid_code(db: PgPool) {
    let c = cfg(&[("REGISTRATION_MODE", "invite")]);
    // No code → rejected.
    assert!(matches!(
        registration::authorize(&db, &c, None).await,
        Err(AppError::BadRequest(_))
    ));
    // Unknown code → rejected.
    assert!(matches!(
        registration::authorize(&db, &c, Some("ghost")).await,
        Err(AppError::BadRequest(_))
    ));

    // A valid invite is redeemed; its created_by becomes invited_by; instance_pin steers placement.
    repository::create_invite(
        &db,
        "good",
        Some(3),
        None,
        "alice",
        Some("pinned.example.com"),
    )
        .await
        .unwrap();
    let auth = registration::authorize(&db, &c, Some("good"))
        .await
        .unwrap();
    assert_eq!(auth.invited_by().as_deref(), Some("alice"));
    assert_eq!(auth.instance_pin(), Some("pinned.example.com"));
    // Use count incremented.
    assert_eq!(
        repository::get_invite(&db, "good")
            .await
            .unwrap()
            .unwrap()
            .uses,
        1
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn registration_invite_max_uses_is_atomic(db: PgPool) {
    let c = cfg(&[("REGISTRATION_MODE", "invite")]);
    repository::create_invite(&db, "one", Some(1), None, "alice", None)
        .await
        .unwrap();
    // First redemption succeeds.
    assert!(registration::authorize(&db, &c, Some("one")).await.is_ok());
    // Second exceeds max_uses → rejected.
    assert!(matches!(
        registration::authorize(&db, &c, Some("one")).await,
        Err(AppError::BadRequest(_))
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn registration_invite_expiry_rejected(db: PgPool) {
    let c = cfg(&[("REGISTRATION_MODE", "invite")]);
    repository::create_invite(
        &db,
        "old",
        None,
        Some(Utc::now() - Duration::seconds(1)),
        "alice",
        None,
    )
        .await
        .unwrap();
    assert!(matches!(
        registration::authorize(&db, &c, Some("old")).await,
        Err(AppError::BadRequest(_))
    ));
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn invite_cleanup_removes_expired_and_exhausted(db: PgPool) {
    repository::create_invite(&db, "live", Some(5), None, "a", None)
        .await
        .unwrap();
    repository::create_invite(
        &db,
        "expired",
        None,
        Some(Utc::now() - Duration::hours(1)),
        "a",
        None,
    )
        .await
        .unwrap();
    repository::create_invite(&db, "spent", Some(1), None, "a", None)
        .await
        .unwrap();
    repository::redeem_invite(&db, "spent").await.unwrap();

    let removed = repository::cleanup_invites(&db).await.unwrap();
    assert_eq!(removed, 2);
    assert!(repository::get_invite(&db, "live").await.unwrap().is_some());
    assert!(
        repository::get_invite(&db, "expired")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repository::get_invite(&db, "spent")
            .await
            .unwrap()
            .is_none()
    );
}

// ── Operator credential (§5.1) ─────────────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn operator_seed_and_login(db: PgPool) {
    let c = cfg(&[("RESOLVER_ADMIN_TOKEN", "s3cret-operator-token")]);
    operator::ensure_seeded(&db, &c).await.unwrap();
    // Idempotent: a second seed is a no-op.
    operator::ensure_seeded(&db, &c).await.unwrap();

    let jwt = JwtService::new(&c.get(sk::RESOLVER_JWT_SECRET), &c.get(sk::GLOBAL_DOMAIN));
    let gd = c.get(sk::GLOBAL_DOMAIN);

    // Wrong token → Unauthorized.
    assert!(matches!(
        operator::login(&db, &jwt, &gd, "wrong").await,
        Err(AppError::Unauthorized(_))
    ));

    // Correct token → a ResolverAdminSession JWT + a working refresh token.
    let session = operator::login(&db, &jwt, &gd, "s3cret-operator-token")
        .await
        .unwrap();
    let claims = jwt.decode_any_issuer(&session.session_token, &gd).unwrap();
    assert_eq!(claims.token_type, TokenType::ResolverAdminSession);
    assert!(claims.is_admin);

    // Refresh rotates: the old refresh token no longer works after a new one is issued.
    let refreshed = operator::refresh(&db, &jwt, &gd, &session.refresh_token)
        .await
        .unwrap();
    let _ = operator::refresh(&db, &jwt, &gd, &refreshed.refresh_token)
        .await
        .unwrap();
    assert!(matches!(
        operator::refresh(&db, &jwt, &gd, &session.refresh_token).await,
        Err(AppError::Unauthorized(_))
    ));
}

// ── Settings engine precedence (§4.2) ──────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn settings_env_locks_and_db_override_wins_when_unlocked(_db: PgPool) {
    // Unset → default.
    let base = cfg(&[]);
    assert_eq!(base.get(sk::PIN_IMPORTANCE), 0);
    assert_eq!(base.get(sk::REGISTRATION_MODE), RegistrationMode::Open);

    // Env-set → locked. A PATCH (validate_override) is rejected before it can reach the DB.
    let locked = cfg(&[("SELECTION_STRATEGY", "least_pictures")]);
    assert!(locked.is_locked_str("selection_strategy"));
    assert_eq!(
        locked.get(sk::SELECTION_STRATEGY),
        SelectionStrategy::LeastPictures
    );
    assert!(
        locked
            .validate_override_str("selection_strategy", &serde_json::json!("least_users"))
            .is_err(),
        "an env-locked field rejects a DB override at PATCH time"
    );

    // Unlocked field → the override validates and, once persisted + reloaded, wins over the default.
    let unlocked = cfg(&[]);
    assert!(!unlocked.is_locked_str("pin_importance"));
    let coerced = unlocked
        .validate_override_str("pin_importance", &serde_json::json!(42))
        .unwrap();
    unlocked
        .reload(&std::collections::HashMap::from([(
            "pin_importance".to_string(),
            coerced,
        )]))
        .unwrap();
    assert_eq!(unlocked.get(sk::PIN_IMPORTANCE), 42);
}

// ── Delegation-replay client: unreachable backends (§3.2, §5.3) ─────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn backend_client_reports_unreachable(db: PgPool) {
    let client = archypix_resolver::clients::BackendClient::new(db.clone(), reqwest::Client::new());

    // Unknown backend → NotFound.
    assert!(matches!(
        client.get_json("ghost.example.com", "/x").await,
        Err(AppError::NotFound)
    ));

    // Registered but never heartbeated (reachable=false, no delegation token) → 503.
    repository::upsert_backend(&db, "silent.example.com", false, "http://silent")
        .await
        .unwrap();
    assert!(matches!(
        client.get_json("silent.example.com", "/x").await,
        Err(AppError::ServiceUnavailable(_))
    ));
}
