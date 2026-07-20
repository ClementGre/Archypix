//! Federation integration suite.
//!
//! Sub-modules:
//!   `contract`  — end-to-end protocol flows with two real Axum servers.
//!   `rejection` — security boundaries and error paths in federation API handlers.
//!   `presign`   — the share-presign endpoint (authorised by share_token, not a JWT).

#[path = "../common/mod.rs"]
mod common;
mod contract;
mod presign;
mod rejection;

// ── Shared infrastructure ─────────────────────────────────────────────────────

use archypix_back::infra::settings::test_settings_with;
use archypix_common::settings::Settings;
use axum::body::Body;
use axum::http::{header, Request};
use serde_json::Value;
use std::sync::Arc;

/// Single-server settings for "backend A" — oneshot tests only.
/// `back_domain` is a static fake hostname; no real port needed.
pub(crate) fn settings_a() -> Arc<Settings> {
    test_settings_with(&[
        ("GLOBAL_DOMAIN", "a.test"),
        ("BACK_DOMAIN", "backend-a.test"),
    ])
}

/// Single-server settings for "backend B" — oneshot tests only.
pub(crate) fn settings_b() -> Arc<Settings> {
    test_settings_with(&[
        ("GLOBAL_DOMAIN", "b.test"),
        ("BACK_DOMAIN", "backend-b.test"),
    ])
}

/// Build a POST request with a federation bearer token.
pub(crate) fn post_fed(path: &str, bearer: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

/// Build a POST request with no authentication header. Carries a stub `ConnectInfo` so IP-based
/// rate limiters (e.g. the federation presign endpoint) can extract a source address under
/// `oneshot` (production supplies it via `into_make_service_with_connect_info`).
pub(crate) fn post_no_auth(path: &str, body: &Value) -> Request<Body> {
    use axum::extract::ConnectInfo;
    use std::net::{Ipv4Addr, SocketAddr};
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .extension(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 12345))))
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

/// Consume a response body and parse it as JSON.
pub(crate) async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}
