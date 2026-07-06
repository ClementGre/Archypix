use super::bearer_token;
use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::settings::keys;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// Extractor for endpoints that require a valid worker JWT.
///
/// Workers authenticate with the same shared-secret pattern as the resolver:
/// `WORKER_JWT_SECRET` is configured identically on the backend and all worker
/// instances. Workers sign a short-lived JWT (`TokenType::Worker`) and present
/// it as a `Bearer` token.
#[derive(Clone, Debug)]
pub struct AuthWorker {
    pub claims: JwtClaims,
}

impl AuthWorker {
    pub fn worker_id(&self) -> &str {
        &self.claims.sub
    }
}

impl FromRequestParts<AppState> for AuthWorker {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        // decode_any_issuer: skip issuer check (worker may be on any host).
        let claims = state
            .worker_jwt
            .decode_any_issuer(&token, &state.settings.get(keys::BACK_DOMAIN))?;

        if claims.token_type != TokenType::Worker {
            return Err(AppError::Unauthorized(
                "Invalid token type for worker access".to_string(),
            ));
        }

        Ok(AuthWorker { claims })
    }
}
