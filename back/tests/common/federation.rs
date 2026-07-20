//! Helpers shared by federation integration tests.
//!
//! Provides:
//! - [`settings_a`] / [`settings_b`] — two-domain test settings
//! - [`spawn_backend`]               — real Axum server on an OS-assigned port
//! - [`make_client`]                 — FederationClient sharing a server's cache
//! - [`seed_backend_url`]            — bypass resolution by pre-seeding the cache
//! - [`federation_jwt`]              — forge a federation JWT for a given server
//! - [`user_jwt`]                    — forge a user access JWT for a given server

use super::InMemoryCache;
use archypix_back::clients::federation::FederationClient;
use archypix_back::domain::auth::TokenType;
use archypix_back::infra::crypto::JwtService;
use archypix_back::infra::redis::{Cache, RedisKey};
use archypix_back::infra::settings::keys;
use archypix_common::settings::Settings;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

// ── Settings ───────────────────────────────────────────────────────────────────

/// Settings for "backend A" (alice's home instance): `global_domain = "a.test"`.
/// `back_domain` is a placeholder replaced by [`spawn_backend`] after binding.
pub fn settings_a() -> Arc<Settings> {
    archypix_back::infra::settings::test_settings_with(&[
        ("GLOBAL_DOMAIN", "a.test"),
        ("BACK_DOMAIN", "a.test:0"),
    ])
}

/// Settings for "backend B" (bob's home instance): `global_domain = "b.test"`.
pub fn settings_b() -> Arc<Settings> {
    archypix_back::infra::settings::test_settings_with(&[
        ("GLOBAL_DOMAIN", "b.test"),
        ("BACK_DOMAIN", "b.test:0"),
    ])
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

/// Spawn a full Axum server on an OS-assigned port.
///
/// Updates `settings.get(keys::BACK_DOMAIN)` to match the bound port so all JWTs issued or
/// verified by this server use the correct audience. Returns
/// `(socket_addr, cache_handle, final_settings)`.
///
/// **Pre-seed the returned cache with [`seed_backend_url`] entries for any
/// remote domain before making federation calls**, so backend resolution is
/// bypassed without a real resolver.
pub async fn spawn_backend(
    db: PgPool,
    mut settings: Arc<Settings>,
) -> (SocketAddr, Arc<InMemoryCache>, Arc<Settings>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();

    // Fix the audience to the real port + a generous federation timeout for slow CI.
    let final_settings = Arc::new(settings.cloned_with(&[
        ("BACK_DOMAIN", format!("127.0.0.1:{}", addr.port())),
        ("FEDERATION_REQUEST_TIMEOUT_MS", "5000".to_string()),
    ]));

    let cache = Arc::new(InMemoryCache::new());
    let cache_dyn: Arc<dyn Cache> = cache.clone();
    let state = super::test_app_state_with_cache(db, &final_settings, cache_dyn);
    let app = archypix_back::api::routes(final_settings.clone()).with_state(state);

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("federation test server crashed");
    });

    (addr, cache, final_settings)
}

// ── Client helper ─────────────────────────────────────────────────────────────

/// Build a `FederationClient` that **shares `cache` with the running server**.
///
/// The cache is the same `Arc` returned by [`spawn_backend`], so tokens written
/// by the server's `/api/federation/auth/grant` handler are immediately visible
/// to this client's poll loop, and backend-URL seeds written via [`seed_backend_url`]
/// are resolved without a real resolver lookup.
pub fn make_client(settings: &Arc<Settings>, cache: &Arc<InMemoryCache>) -> FederationClient {
    FederationClient::new(
        reqwest::Client::new(),
        settings.clone(),
        JwtService::new(
            &settings.get(keys::JWT_SECRET),
            &settings.get(keys::BACK_DOMAIN),
        ),
        cache.clone() as Arc<dyn Cache>,
    )
}

// ── Cache helpers ─────────────────────────────────────────────────────────────

/// Pre-seed `cache` so that `FederationClient::resolve_backend_url(username, domain)`
/// returns `backend_url` immediately without making a resolver HTTP call.
///
/// Call this for every `(username, domain)` pair that a server will need to
/// resolve before the test makes federation requests.
pub async fn seed_backend_url(
    cache: &InMemoryCache,
    username: &str,
    domain: &str,
    backend_url: &str,
) {
    cache
        .set_str_ex(
            RedisKey::FederationBackend(username, domain),
            backend_url,
            3_600,
        )
        .await
        .unwrap();
}

/// Seed a pending federation-handshake nonce for `domain`, simulating an outbound `auth/request`
/// this backend sent. Lets a test exercise the `auth/grant` nonce-acceptance path directly.
pub async fn seed_auth_nonce(cache: &InMemoryCache, domain: &str, nonce: &str) {
    cache
        .set_str_ex(RedisKey::FederationAuthNonce(domain), nonce, 60)
        .await
        .unwrap();
}

// ── JWT helpers ───────────────────────────────────────────────────────────────

/// Issue a federation JWT that `settings`'s auth middleware would accept.
///
/// Mirrors what `FederationClient::issue_federation_token` produces on the server:
/// signed with the server's `jwt_secret`, audience = `back_domain`,
/// subject = `authenticated_as` (the calling instance's global domain).
pub fn federation_jwt(settings: &Arc<Settings>, authenticated_as: &str) -> String {
    let jwt = JwtService::new(
        &settings.get(keys::JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );
    jwt.issue(
        authenticated_as,
        None,
        &settings.get(keys::GLOBAL_DOMAIN),
        TokenType::Federation,
        false,
        &settings.get(keys::BACK_DOMAIN),
        3_600,
    )
    .unwrap()
}

/// Issue a user access JWT accepted by a server running `settings`.
pub fn user_jwt(settings: &Settings, username: &str, user_id: Uuid) -> String {
    let jwt = JwtService::new(
        &settings.get(keys::JWT_SECRET),
        &settings.get(keys::BACK_DOMAIN),
    );
    jwt.issue(
        username,
        Some(user_id),
        &settings.get(keys::GLOBAL_DOMAIN),
        TokenType::User,
        false,
        &settings.get(keys::BACK_DOMAIN),
        900,
    )
    .unwrap()
}
