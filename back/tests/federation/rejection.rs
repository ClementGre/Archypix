//! Security boundary tests for federation API handlers.
//!
//! Each test injects a malformed or unauthorised request directly into a single
//! in-process router via `oneshot` and asserts the correct rejection status.
//! No second server is needed — these paths are purely receiver-side.
//!
//! Invariants covered:
//!   • wrong `recipient_instance`        → 400
//!   • JWT `sub` ≠ `sender_instance`     → 401
//!   • JWT `sub` ≠ share recipient       → 401
//!   • unknown share / user              → 404
//!   • pictures on a Pending share       → 404

use crate::common;
use crate::{cfg_a, cfg_b, post_fed};

use archypix_back::repository::share::{IncomingShareRepository, OutgoingShareRepository};
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub(crate) static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// ── /api/federation/shares/announce ──────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn announce_share_rejects_wrong_recipient_instance(db: PgPool) {
    let cfg = cfg_b();
    common::seed_user(&db, "bob", "pass").await;
    let token = common::federation::federation_jwt(&cfg, "a.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/shares/announce",
            &token,
            &json!({
                "sender_username": "alice",   "sender_instance":    "a.test",
                "recipient_username": "bob",  "recipient_instance": "wrong.com",
                "outgoing_share_id": Uuid::new_v4(), "tag_path": "vacation",
                "name": "Test share", "message": null,
                "allow_share_back": false, "future": false,
                "shareback_of": null
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn announce_share_rejects_sender_instance_mismatch(db: PgPool) {
    let cfg = cfg_b();
    common::seed_user(&db, "bob", "pass").await;
    // JWT sub is "c.test" but payload claims sender_instance "a.test".
    let token = common::federation::federation_jwt(&cfg, "c.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/shares/announce",
            &token,
            &json!({
                "sender_username": "alice",  "sender_instance":    "a.test",
                "recipient_username": "bob", "recipient_instance": "b.test",
                "outgoing_share_id": Uuid::new_v4(), "tag_path": "vacation",
                "name": "Test share", "message": null,
                "allow_share_back": false, "future": false,
                "shareback_of": null
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn announce_share_rejects_unknown_recipient(db: PgPool) {
    let cfg = cfg_b();
    let token = common::federation::federation_jwt(&cfg, "a.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/shares/announce",
            &token,
            &json!({
                "sender_username": "alice",    "sender_instance":    "a.test",
                "recipient_username": "nobody", "recipient_instance": "b.test",
                "outgoing_share_id": Uuid::new_v4(), "tag_path": "vacation",
                "name": "Test share", "message": null,
                "allow_share_back": false, "future": false,
                "shareback_of": null
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── /api/federation/shares/revoke ────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn revoke_share_not_found_for_unknown_id(db: PgPool) {
    let cfg = cfg_b();
    let token = common::federation::federation_jwt(&cfg, "a.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/shares/revoke",
            &token,
            &json!({ "outgoing_share_id": Uuid::new_v4() }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── /api/federation/shares/reject ────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn reject_share_rejects_instance_mismatch(db: PgPool) {
    let cfg = cfg_a();
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let share = OutgoingShareRepository::create(
        &db,
        alice_id,
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        true,
        false,
        None,
    )
    .await
    .unwrap();

    // JWT sub "c.test" ≠ share.recipient_instance "b.test".
    let token = common::federation::federation_jwt(&cfg, "c.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/shares/reject",
            &token,
            &json!({ "outgoing_share_id": share.id }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── /api/federation/shares/accept ────────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn accept_share_rejects_instance_mismatch(db: PgPool) {
    let cfg = cfg_a();
    let alice_id = common::seed_user(&db, "alice", "pass").await;
    let share = OutgoingShareRepository::create(
        &db,
        alice_id,
        "vacation",
        "Test share",
        None,
        "bob",
        "b.test",
        true,
        false,
        None,
    )
    .await
    .unwrap();

    // JWT sub "c.test" ≠ share.recipient_instance "b.test".
    let token = common::federation::federation_jwt(&cfg, "c.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/shares/accept",
            &token,
            &json!({ "outgoing_share_id": share.id }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── /api/federation/pictures/announce ────────────────────────────────────────

#[sqlx::test(migrator = "MIGRATOR")]
async fn announce_pictures_rejects_pending_share(db: PgPool) {
    let cfg = cfg_b();
    let bob_id = common::seed_user(&db, "bob", "pass").await;
    let outgoing_id = Uuid::new_v4();

    // Share is still Pending — pictures must be refused until Bob accepts.
    IncomingShareRepository::create(
        &db,
        bob_id,
        "alice",
        "a.test",
        "Test share",
        None,
        outgoing_id,
        false,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    let token = common::federation::federation_jwt(&cfg, "a.test");
    let app = archypix_back::api::routes(&cfg).with_state(common::test_app_state(db.clone(), &cfg));

    let resp = app
        .oneshot(post_fed(
            "/api/federation/pictures/announce",
            &token,
            &json!({
                "outgoing_share_id": outgoing_id,
                "tag_path": "vacation",
                "sender_username": "alice", "sender_instance": "a.test",
                "pictures": [{
                    "picture_id": Uuid::new_v4().to_string(),
                    "owner_username": "alice", "owner_instance_domain": "a.test",
                    "picture_token": Uuid::new_v4(),
                    "filename": null, "mime_type": null,
                    "file_size": null, "width": null, "height": null, "captured_at": null
                }]
            }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
