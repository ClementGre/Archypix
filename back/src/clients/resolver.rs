use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::crypto::JwtService;
use crate::infra::settings::keys;
use archypix_common::error::AppError;
use archypix_common::registration::{Invite, RegistrationMode};
use archypix_common::settings::Settings;
use chrono::{DateTime, Utc};
use reqwest::Client as HttpClient;
use serde::Serialize;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Outbound client for the Resolver service.
#[derive(Clone)]
pub struct ResolverClient {
    http: HttpClient,
    settings: Arc<Settings>,
    /// Signs/verifies the shared-secret `Resolver` **push** tokens (self-register, update, heartbeat).
    jwt: JwtService,
    /// The backend's own token authority (signed with `JWT_SECRET`) — mints the `ResolverDelegation`
    /// token the resolver replays back at this backend (feature 23 §3.2).
    back_jwt: JwtService,
}

impl ResolverClient {
    pub fn new(
        http: HttpClient,
        settings: Arc<Settings>,
        jwt: JwtService,
        back_jwt: JwtService,
    ) -> Self {
        Self {
            http,
            settings,
            jwt,
            back_jwt,
        }
    }

    /// Verify an inbound resolver push JWT (shared-secret `Resolver` token).
    pub fn verify_token(&self, token: &str) -> Result<JwtClaims, AppError> {
        self.jwt
            .decode_any_issuer(token, &self.settings.get(keys::BACK_DOMAIN))
            .map_err(Into::into)
    }

    /// Mint a fresh backend-signed `ResolverDelegation` token (feature 23 §3.2): `is_admin`,
    /// `iss = aud = back_domain`, `sub = "resolver"`. The resolver replays it as `Bearer` on every
    /// call it makes to this backend; the backend verifies it with its own `JWT_SECRET`.
    pub fn mint_delegation_token(&self) -> Result<String, AppError> {
        self.back_jwt
            .issue(
                "resolver",
                None,
                &self.settings.get(keys::GLOBAL_DOMAIN),
                TokenType::ResolverDelegation,
                true,
                &self.settings.get(keys::BACK_DOMAIN),
                self.settings.get(keys::RESOLVER_DELEGATION_TTL_SECS),
            )
            .map_err(Into::into)
    }

    /// Push a heartbeat to the resolver: a fresh delegation token + fleet metrics (feature 23 §3.2).
    /// Authenticated by a shared-secret `Resolver` push token. No-op when `use_resolver=false`.
    #[tracing::instrument(skip(self, metrics), fields(otel.kind = "client", back_domain = %self.settings.get(keys::BACK_DOMAIN)))]
    pub async fn heartbeat(&self, metrics: HeartbeatMetrics) -> Result<(), AppError> {
        if !self.settings.get(keys::USE_RESOLVER) {
            return Ok(());
        }
        let delegation_token = self.mint_delegation_token()?;
        let push_token = self.jwt.issue(
            "heartbeat",
            None,
            &self.settings.get(keys::BACK_DOMAIN),
            TokenType::Resolver,
            false,
            &self.settings.get(keys::GLOBAL_DOMAIN),
            300,
        )?;
        let url = self.resolver_url("/api/backends/heartbeat");
        self.http
            .post(&url)
            .bearer_auth(push_token)
            .json(&HeartbeatRequest {
                back_domain: self.settings.get(keys::BACK_DOMAIN).clone(),
                delegation_token,
                user_count: metrics.user_count,
                picture_count: metrics.picture_count,
                storage_bytes: metrics.storage_bytes,
                healthy: metrics.healthy,
                version: env!("CARGO_PKG_VERSION").to_string(),
            })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "resolver: heartbeat request failed");
                AppError::InternalServerError(format!("Resolver heartbeat: {e}"))
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(format!("Resolver heartbeat: {e}")))?;
        debug!("resolver: heartbeat delivered");
        Ok(())
    }

    /// Register this backend with the resolver at startup. No-op when `use_resolver=false`.
    ///
    /// Sends `back_domain`, `use_https`, and `internal_url` so the resolver can:
    /// - Return the correct public URL in WebFinger responses.
    /// - Use the internal URL to forward user registration requests.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", back_domain = %self.settings.get(keys::BACK_DOMAIN), resolver_url))]
    pub async fn self_register(&self) -> Result<(), AppError> {
        if !self.settings.get(keys::USE_RESOLVER) {
            debug!("resolver: use_resolver=false, skipping self-registration");
            return Ok(());
        }

        let token = self.jwt.issue(
            "self-register",
            None,
            &self.settings.get(keys::BACK_DOMAIN),
            TokenType::Resolver,
            false,
            &self.settings.get(keys::GLOBAL_DOMAIN),
            300,
        )?;

        let url = self.resolver_url("/api/backends");
        tracing::Span::current().record("resolver_url", url.as_str());

        let internal_url = self.settings.get(keys::BACK_INTERNAL_URL);

        self.http
            .post(&url)
            .bearer_auth(token)
            .json(&SelfRegisterRequest {
                back_domain: self.settings.get(keys::BACK_DOMAIN).clone(),
                use_https: self.settings.get(keys::BACK_USE_HTTPS),
                internal_url: internal_url.clone(),
            })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "resolver: self-registration request failed");
                AppError::InternalServerError(format!("Resolver self-register: {e}"))
            })?
            .error_for_status()
            .map_err(|e| {
                warn!(error = %e, "resolver: self-registration rejected");
                AppError::InternalServerError(format!("Resolver self-register: {e}"))
            })?;

        info!(internal_url, "Registered with resolver");
        Ok(())
    }

    /// Mint a short-lived shared-secret `Resolver` push token (feature 23 §3.1 row 1).
    fn push_token(&self) -> Result<String, AppError> {
        self.jwt
            .issue(
                "resolver-push",
                None,
                &self.settings.get(keys::BACK_DOMAIN),
                TokenType::Resolver,
                false,
                &self.settings.get(keys::GLOBAL_DOMAIN),
                300,
            )
            .map_err(Into::into)
    }

    /// Build a URL to the resolver, prepending the `/archypix-resolver` mount prefix (feature 25) to
    /// the internal base so every backend→resolver call hits the nested router.
    fn resolver_url(&self, path: &str) -> String {
        format!(
            "{}/archypix-resolver{}",
            self.settings
                .get(keys::RESOLVER_INTERNAL_URL)
                .trim_end_matches('/'),
            path
        )
    }

    /// The resolver's **effective** registration mode (authoritative behind a resolver — the backend's
    /// own `registration_mode` setting is standalone-only). Read from the public registration-info.
    #[tracing::instrument(skip(self), fields(otel.kind = "client"))]
    pub async fn registration_mode(&self) -> Result<RegistrationMode, AppError> {
        #[derive(serde::Deserialize)]
        struct Info {
            mode: RegistrationMode,
        }
        let resp = self
            .http
            .get(self.resolver_url("/api/public/registration-info"))
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver registration_mode: {e}")))?
            .error_for_status()
            .map_err(|e| {
                AppError::InternalServerError(format!("Resolver registration_mode: {e}"))
            })?;
        Ok(resp
            .json::<Info>()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver registration_mode: {e}")))?
            .mode)
    }

    /// Ask the resolver to mint an invite on the backend's behalf (feature 23 §6.2 — in resolver mode
    /// invites live in the resolver's DB). `instance_pin` steers the invitee back to this backend.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", created_by = %created_by))]
    pub async fn create_invite(
        &self,
        created_by: &str,
        max_uses: Option<i64>,
        expires_at: Option<DateTime<Utc>>,
        instance_pin: Option<&str>,
    ) -> Result<Invite, AppError> {
        let resp = self
            .http
            .post(self.resolver_url("/api/backends/invites"))
            .bearer_auth(self.push_token()?)
            .json(&CreateInviteRequest {
                created_by: created_by.to_string(),
                max_uses,
                expires_at,
                instance_pin: instance_pin.map(str::to_string),
            })
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver create_invite: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(format!("Resolver create_invite: {e}")))?;
        resp.json::<Invite>()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver create_invite: {e}")))
    }

    /// List the invites minted by `created_by` (resolver mode).
    #[tracing::instrument(skip(self), fields(otel.kind = "client", created_by = %created_by))]
    pub async fn list_invites(&self, created_by: &str) -> Result<Vec<Invite>, AppError> {
        let resp = self
            .http
            .get(self.resolver_url("/api/backends/invites"))
            .query(&[("created_by", created_by)])
            .bearer_auth(self.push_token()?)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver list_invites: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(format!("Resolver list_invites: {e}")))?;
        resp.json::<Vec<Invite>>()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver list_invites: {e}")))
    }

    /// Revoke a resolver-stored invite.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", code = %code))]
    pub async fn delete_invite(&self, code: &str) -> Result<(), AppError> {
        self.http
            .delete(self.resolver_url(&format!("/api/backends/invites/{code}")))
            .bearer_auth(self.push_token()?)
            .send()
            .await
            .map_err(|e| AppError::InternalServerError(format!("Resolver delete_invite: {e}")))?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(format!("Resolver delete_invite: {e}")))?;
        Ok(())
    }

    /// Register or update the username→backend mapping in the resolver.
    /// No-op when `use_resolver=false`.
    #[tracing::instrument(skip(self), fields(otel.kind = "client", username = %username, back_domain = %self.settings.get(keys::BACK_DOMAIN)))]
    pub async fn update_mapping(&self, username: &str) -> Result<(), AppError> {
        if !self.settings.get(keys::USE_RESOLVER) {
            return Ok(());
        }
        debug!("resolver: update_mapping");

        let token = self.jwt.issue(
            "resolver-update",
            None,
            &self.settings.get(keys::BACK_DOMAIN),
            TokenType::Resolver,
            false,
            &self.settings.get(keys::GLOBAL_DOMAIN),
            300,
        )?;

        let url = self.resolver_url("/api/update");

        self.http
            .post(&url)
            .bearer_auth(token)
            .json(&UpdateMappingRequest {
                username: username.to_string(),
                back_domain: self.settings.get(keys::BACK_DOMAIN).clone(),
            })
            .send()
            .await
            .map_err(|e| {
                warn!(error = %e, "resolver: update_mapping request failed");
                AppError::InternalServerError(e.to_string())
            })?
            .error_for_status()
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        Ok(())
    }
}

/// Fleet metrics gathered per heartbeat (feature 23 §3.2), used by the resolver's placement
/// strategies and overview dashboard.
#[derive(Debug, Clone, Copy)]
pub struct HeartbeatMetrics {
    pub user_count: i64,
    pub picture_count: i64,
    pub storage_bytes: i64,
    pub healthy: bool,
}

#[derive(Serialize)]
struct HeartbeatRequest {
    back_domain: String,
    delegation_token: String,
    user_count: i64,
    picture_count: i64,
    storage_bytes: i64,
    healthy: bool,
    version: String,
}

#[derive(Serialize)]
struct SelfRegisterRequest {
    back_domain: String,
    use_https: bool,
    internal_url: String,
}

#[derive(Serialize)]
struct UpdateMappingRequest {
    username: String,
    back_domain: String,
}

#[derive(Serialize)]
struct CreateInviteRequest {
    created_by: String,
    max_uses: Option<i64>,
    expires_at: Option<DateTime<Utc>>,
    instance_pin: Option<String>,
}
