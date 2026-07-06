//! Resolver configuration — the single source of truth, on the shared [`Settings`] engine (feature
//! 23 §4), same as the backend. Core fields (DB, secrets, topology) are env-only; operational fields
//! (selection strategy, registration mode, routine intervals, CORS) are DB-editable from the fleet
//! dashboard and read live. Read via `config.get(setting_keys::GLOBAL_DOMAIN)`.

use archypix_common::registration::RegistrationMode;
use archypix_common::settings::{SettingKey, SettingSpec, Settings};
use archypix_common::wire_enum;
use std::collections::HashMap;
use std::sync::Arc;

pub type Config = Arc<Settings>;

/// New-user placement strategy across backends (feature 23 §7).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    strum::VariantNames,
    strum::EnumString,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SelectionStrategy {
    LeastUsers,
    LeastPictures,
    LeastStorage,
    RoundRobin,
    Static,
}
wire_enum!(SelectionStrategy);

pub mod group {
    pub const SERVER: &str = "Server";
    pub const DATABASE: &str = "Database";
    pub const IDENTITY: &str = "Identity";
    pub const AUTH: &str = "Authentication";
    pub const PLACEMENT: &str = "Placement";
    pub const REGISTRATION: &str = "Registration";
    pub const ROUTINES: &str = "Routines";
    pub const CACHE: &str = "Cache";
}

pub mod setting_keys {
    use super::*;

    // ── Core ──
    pub const LISTEN_ADDR: SettingKey<String> = SettingKey::new("listen_addr");
    pub const DB_HOST: SettingKey<String> = SettingKey::new("db_host");
    pub const DB_PORT: SettingKey<u16> = SettingKey::new("db_port");
    pub const DB_USER: SettingKey<String> = SettingKey::new("db_user");
    pub const DB_PASSWORD: SettingKey<Option<String>> = SettingKey::new("db_password");
    pub const DB_NAME: SettingKey<String> = SettingKey::new("db_name");
    pub const GLOBAL_DOMAIN: SettingKey<String> = SettingKey::new("global_domain");
    /// Shared secret authenticating backend→resolver pushes; also signs operator session tokens.
    pub const RESOLVER_JWT_SECRET: SettingKey<String> = SettingKey::new("resolver_jwt_secret");
    /// Operator dashboard token (plaintext or argon2 hash). Unset ⇒ generated + printed once at boot.
    pub const RESOLVER_ADMIN_TOKEN: SettingKey<Option<String>> =
        SettingKey::new("resolver_admin_token");
    pub const CACHE_TTL_SECS: SettingKey<u64> = SettingKey::new("cache_ttl_secs");
    pub const CACHE_MAX_CAPACITY: SettingKey<u64> = SettingKey::new("cache_max_capacity");

    // ── Runtime (dashboard-editable) ──
    pub const CORS_ORIGINS: SettingKey<Vec<String>> = SettingKey::new("cors_origins");
    pub const SELECTION_STRATEGY: SettingKey<SelectionStrategy> =
        SettingKey::new("selection_strategy");
    pub const STATIC_BACKEND: SettingKey<Option<String>> = SettingKey::new("static_backend");
    pub const PIN_IMPORTANCE: SettingKey<i64> = SettingKey::new("pin_importance");
    pub const REGISTRATION_MODE: SettingKey<RegistrationMode> =
        SettingKey::new("registration_mode");
    pub const DELEGATION_STALE_SECS: SettingKey<u64> = SettingKey::new("delegation_stale_secs");
    pub const STALE_PRUNE_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("stale_prune_interval_secs");
    pub const INVITE_CLEANUP_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("invite_cleanup_interval_secs");
    pub const MAPPING_RECONCILE_INTERVAL_SECS: SettingKey<u64> =
        SettingKey::new("mapping_reconcile_interval_secs");
}

pub fn registry() -> Vec<SettingSpec> {
    use setting_keys::*;
    vec![
        SettingSpec::new(LISTEN_ADDR, group::SERVER).core().default("0.0.0.0:80").doc("HTTP bind address.", "0.0.0.0:8080"),
        SettingSpec::new(DB_HOST, group::DATABASE).core().doc("Postgres host.", "postgres"),
        SettingSpec::new(DB_PORT, group::DATABASE).core().default("5432").doc("Postgres port.", "5432"),
        SettingSpec::new(DB_USER, group::DATABASE).core().default("postgres").doc("Postgres user.", "archypix"),
        SettingSpec::new(DB_PASSWORD, group::DATABASE).secret().nullable().doc("Postgres password.", ""),
        SettingSpec::new(DB_NAME, group::DATABASE).core().default("archypix_resolver").doc("Postgres database name.", "archypix_resolver"),
        SettingSpec::new(GLOBAL_DOMAIN, group::IDENTITY).core().doc("The global identity domain this resolver fronts — the part after ':' in @user:global_domain. Must match GLOBAL_DOMAIN on every backend that registers here.", "example.com"),
        SettingSpec::new(RESOLVER_JWT_SECRET, group::AUTH).secret().doc("Shared HS256 secret authenticating backend→resolver PUSHES (self-register / mapping update / heartbeat) and signing operator dashboard sessions. Every registered backend must set this same value as its RESOLVER_JWT_SECRET.", ""),
        SettingSpec::new(RESOLVER_ADMIN_TOKEN, group::AUTH).secret().nullable().doc("Operator token for the fleet dashboard (plaintext or an argon2 hash). If unset, one is generated and printed to the console ONCE at first startup. Stored hashed; rotatable from the dashboard unless env-set.", ""),
        SettingSpec::new(CACHE_TTL_SECS, group::CACHE).core().default("3600").doc("username→backend cache TTL (seconds).", "3600"),
        SettingSpec::new(CACHE_MAX_CAPACITY, group::CACHE).core().default("100000").doc("username→backend cache capacity.", "100000"),
        SettingSpec::new(CORS_ORIGINS, group::SERVER).default("").doc("Allowed CORS origins ('*' = any, dev only). Hot-swapped.", "https://app.example.com"),
        SettingSpec::new(SELECTION_STRATEGY, group::PLACEMENT).default("least_users").doc("How a new user is placed across backends (metric strategies read heartbeat metrics).", "least_users"),
        SettingSpec::new(STATIC_BACKEND, group::PLACEMENT).nullable().doc("The pinned back_domain used by the 'static' strategy.", "backend1.example.com"),
        SettingSpec::new(PIN_IMPORTANCE, group::PLACEMENT).default("0").doc("Weight of an invite's instance_pin (metric-units delta; round-robin/static: ≥1 follows the pin).", "100"),
        SettingSpec::new(REGISTRATION_MODE, group::REGISTRATION).default("open").doc("Who may register across the fleet: open (anyone; a single tracking referral link per user), invite (any user mints invitations), admin_invite (only operators/admins mint).", "open"),
        SettingSpec::new(DELEGATION_STALE_SECS, group::ROUTINES).default("360").doc("A backend is marked unreachable once its delegation token is this many seconds old.", "360"),
        SettingSpec::new(STALE_PRUNE_INTERVAL_SECS, group::ROUTINES).default("60").routine("stale_backend_prune").doc("How often the stale-backend prune runs.", "60"),
        SettingSpec::new(INVITE_CLEANUP_INTERVAL_SECS, group::ROUTINES).default("3600").routine("invite_cleanup").doc("How often expired/exhausted invites are deleted.", "3600"),
        SettingSpec::new(MAPPING_RECONCILE_INTERVAL_SECS, group::ROUTINES).default("3600").routine("mapping_reconcile").doc("How often username→backend mappings are reconciled against each backend's authoritative user list (fixes drift: deleted/moved users the push protocol missed).", "3600"),
    ]
}

// ── Derived ──────────────────────────────────────────────────────────────────────

pub fn database_url(s: &Settings) -> String {
    build_pg_url(
        &s.get(setting_keys::DB_HOST),
        s.get(setting_keys::DB_PORT),
        &s.get(setting_keys::DB_USER),
        s.get(setting_keys::DB_PASSWORD).as_deref(),
        &s.get(setting_keys::DB_NAME),
    )
}
pub fn database_url_masked(s: &Settings) -> String {
    let pw = if s.get(setting_keys::DB_PASSWORD).is_some() {
        Some("***")
    } else {
        None
    };
    build_pg_url(
        &s.get(setting_keys::DB_HOST),
        s.get(setting_keys::DB_PORT),
        &s.get(setting_keys::DB_USER),
        pw,
        &s.get(setting_keys::DB_NAME),
    )
}
fn build_pg_url(host: &str, port: u16, user: &str, password: Option<&str>, db: &str) -> String {
    match password {
        Some(pw) => format!("postgres://{user}:{pw}@{host}:{port}/{db}"),
        None => format!("postgres://{user}@{host}:{port}/{db}"),
    }
}

pub fn load_env_only() -> anyhow::Result<Config> {
    dotenvy::dotenv().ok();
    Ok(Arc::new(Settings::load(&registry(), &HashMap::new())?))
}

pub async fn reload_from_db(settings: &Settings, db: &sqlx::PgPool) -> anyhow::Result<()> {
    let overrides = crate::repository::load_settings_overrides(db).await?;
    settings.reload(&overrides)?;
    Ok(())
}

/// Build a `Config` for tests: the registry with a minimal core-field env layer, plus any
/// `overrides` (env-name → value). No dotenv, no process env, no DB layer.
pub fn test_settings_with(overrides: &[(&str, &str)]) -> Config {
    let mut env: HashMap<String, String> = [
        ("DB_HOST", "localhost"),
        ("GLOBAL_DOMAIN", "example.com"),
        (
            "RESOLVER_JWT_SECRET",
            "test_resolver_secret_must_be_long_enough_for_hs256",
        ),
    ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for (k, v) in overrides {
        env.insert(k.to_string(), v.to_string());
    }
    let lookup = move |k: &str| env.get(k).cloned();
    Arc::new(
        Settings::load_with_env(&registry(), &HashMap::new(), &lookup)
            .expect("test settings must load"),
    )
}
