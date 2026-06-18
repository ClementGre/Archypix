//! Helpers shared by federation integration tests.
//!
//! Provides:
//! - [`config_a`] / [`config_b`]   — two-domain test configurations
//! - [`spawn_backend`]             — real Axum server on an OS-assigned port
//! - [`make_client`]               — FederationClient sharing a server's cache
//! - [`seed_backend_url`]          — bypass WebFinger by pre-seeding the cache
//! - [`federation_jwt`]            — forge a federation JWT for a given server
//! - [`user_jwt`]                  — forge a user access JWT for a given server

use archypix_back::clients::federation::FederationClient;
use archypix_back::domain::auth::TokenType;
use archypix_back::infra::config::Config;
use archypix_back::infra::crypto::JwtService;
use archypix_back::infra::redis::{Cache, RedisKey};
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

use super::InMemoryCache;

// ── Configs ───────────────────────────────────────────────────────────────────

/// Config for "backend A" (alice's home instance): `global_domain = "a.test"`.
/// `back_domain` is a placeholder replaced by [`spawn_backend`] after binding.
pub fn config_a() -> Config {
    Config {
        global_domain: "a.test".to_string(),
        back_domain: "a.test:0".to_string(),
        ..Config::test_defaults()
    }
}

/// Config for "backend B" (bob's home instance): `global_domain = "b.test"`.
pub fn config_b() -> Config {
    Config {
        global_domain: "b.test".to_string(),
        back_domain: "b.test:0".to_string(),
        ..Config::test_defaults()
    }
}

// ── Server lifecycle ──────────────────────────────────────────────────────────

/// Spawn a full Axum server on an OS-assigned port.
///
/// Updates `config.back_domain` to match the bound port so all JWTs issued or
/// verified by this server use the correct audience. Returns
/// `(socket_addr, cache_handle, final_config)`.
///
/// **Pre-seed the returned cache with [`seed_backend_url`] entries for any
/// remote domain before making federation calls**, so WebFinger resolution is
/// bypassed without a real resolver.
pub async fn spawn_backend(
    db: PgPool,
    mut config: Config,
) -> (SocketAddr, Arc<InMemoryCache>, Config) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().unwrap();

    // Fix the audience to the real port.
    config.back_domain = format!("127.0.0.1:{}", addr.port());
    // Generous federation timeout to survive slow CI machines.
    config.federation_request_timeout_ms = 5_000;

    let cache = Arc::new(InMemoryCache::new());
    let cache_dyn: Arc<dyn Cache> = cache.clone();
    let state = super::test_app_state_with_cache(db, &config, cache_dyn);
    let app = archypix_back::api::routes(&config).with_state(state);

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("federation test server crashed");
    });

    (addr, cache, config)
}

// ── Client helper ─────────────────────────────────────────────────────────────

/// Build a `FederationClient` that **shares `cache` with the running server**.
///
/// The cache is the same `Arc` returned by [`spawn_backend`], so tokens written
/// by the server's `/api/federation/auth/grant` handler are immediately visible
/// to this client's poll loop, and backend-URL seeds written via [`seed_backend_url`]
/// are resolved without a real WebFinger lookup.
pub fn make_client(config: &Config, cache: &Arc<InMemoryCache>) -> FederationClient {
    FederationClient::new(
        reqwest::Client::new(),
        config.clone(),
        JwtService::new(&config.jwt_secret, &config.back_domain),
        cache.clone() as Arc<dyn Cache>,
    )
}

// ── Cache helpers ─────────────────────────────────────────────────────────────

/// Pre-seed `cache` so that `FederationClient::resolve_backend_url(username, domain)`
/// returns `backend_url` immediately without making a WebFinger HTTP call.
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

/// Issue a federation JWT that `server_config`'s auth middleware would accept.
///
/// Mirrors what `FederationClient::issue_federation_token` produces on the server:
/// signed with the server's `jwt_secret`, audience = `back_domain`,
/// subject = `authenticated_as` (the calling instance's global domain).
pub fn federation_jwt(server_config: &Config, authenticated_as: &str) -> String {
    let jwt = JwtService::new(&server_config.jwt_secret, &server_config.back_domain);
    jwt.issue(
        authenticated_as,
        None,
        &server_config.global_domain,
        TokenType::Federation,
        false,
        &server_config.back_domain,
        3_600,
    )
    .unwrap()
}

/// Issue a user access JWT accepted by a server running `server_config`.
pub fn user_jwt(server_config: &Config, username: &str, user_id: Uuid) -> String {
    let jwt = JwtService::new(&server_config.jwt_secret, &server_config.back_domain);
    jwt.issue(
        username,
        Some(user_id),
        &server_config.global_domain,
        TokenType::User,
        false,
        &server_config.back_domain,
        900,
    )
    .unwrap()
}
