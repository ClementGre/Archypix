use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::config::Config;
use crate::infra::crypto::JwtService;
use crate::infra::error::AppError;
use reqwest::Client as HttpClient;
use serde::Serialize;
use tracing::{debug, info, warn};

/// Outbound client for the Resolver service.
#[derive(Clone)]
pub struct ResolverClient {
    http: HttpClient,
    config: Config,
    jwt: JwtService,
}

impl ResolverClient {
    pub fn new(http: HttpClient, config: Config, jwt: JwtService) -> Self {
        Self { http, config, jwt }
    }

    /// Verify an inbound resolver JWT (used by the resolver auth middleware).
    pub fn verify_token(&self, token: &str) -> Result<JwtClaims, AppError> {
        self.jwt.decode_any_issuer(token, &self.config.back_domain)
    }

    /// Register this backend with the resolver at startup. No-op when `use_resolver=false`.
    ///
    /// Sends `back_domain`, `use_https`, and `internal_url` so the resolver can:
    /// - Return the correct public URL in WebFinger responses.
    /// - Use the internal URL to forward user registration requests.
    #[tracing::instrument(skip(self), fields(back_domain = %self.config.back_domain, resolver_url))]
    pub async fn self_register(&self) -> Result<(), AppError> {
        if !self.config.use_resolver {
            debug!("resolver: use_resolver=false, skipping self-registration");
            return Ok(());
        }

        let token = self.jwt.issue(
            "self-register",
            None,
            &self.config.back_domain,
            TokenType::Resolver,
            false,
            &self.config.global_domain,
            300,
        )?;

        let url = format!(
            "{}/api/backends",
            self.config.resolver_internal_url.trim_end_matches('/')
        );
        tracing::Span::current().record("resolver_url", url.as_str());

        let internal_url = self
            .config
            .back_internal_url
            .clone()
            .unwrap_or_else(|| self.config.public_base_url());

        self.http
            .post(&url)
            .bearer_auth(token)
            .json(&SelfRegisterRequest {
                back_domain: self.config.back_domain.clone(),
                use_https: self.config.back_use_https,
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

    /// Register or update the username→backend mapping in the resolver.
    /// No-op when `use_resolver=false`.
    #[tracing::instrument(skip(self), fields(username = %username, back_domain = %self.config.back_domain))]
    pub async fn update_mapping(&self, username: &str) -> Result<(), AppError> {
        if !self.config.use_resolver {
            return Ok(());
        }
        debug!("resolver: update_mapping");

        let token = self.jwt.issue(
            "resolver-update",
            None,
            &self.config.back_domain,
            TokenType::Resolver,
            false,
            &self.config.global_domain,
            300,
        )?;

        let url = format!(
            "{}/api/update",
            self.config.resolver_internal_url.trim_end_matches('/')
        );

        self.http
            .post(&url)
            .bearer_auth(token)
            .json(&UpdateMappingRequest {
                username: username.to_string(),
                back_domain: self.config.back_domain.clone(),
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
