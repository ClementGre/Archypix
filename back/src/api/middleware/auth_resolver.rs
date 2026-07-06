use super::bearer_token;
use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::settings::keys;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// Guard for the backend's `/api/resolver/*` provisioning endpoints.
///
/// Feature 23 §3.3 makes the backend-signed **`ResolverDelegation`** token the canonical credential
/// (the resolver replays the token this backend handed it via the heartbeat). The legacy
/// shared-secret **`Resolver`** push token is still accepted as a fallback so a resolver that hasn't
/// yet received a delegation token (first boot, pre-heartbeat) keeps working.
#[derive(Clone, Debug)]
pub struct AuthResolver {
    pub claims: JwtClaims,
}

impl FromRequestParts<AppState> for AuthResolver {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !state.settings.get(keys::USE_RESOLVER) {
            return Err(AppError::Unauthorized(
                "Resolver is disabled on this backend. Set USE_RESOLVER=true and RESOLVER_JWT_SECRET to enable it.".to_string(),
            ));
        }

        let token = bearer_token(&parts.headers)?;

        // Preferred: a backend-signed delegation token (verified with this backend's own JWT_SECRET).
        if let Ok(claims) = state
            .jwt
            .decode(&token, &state.settings.get(keys::BACK_DOMAIN))
        {
            if claims.token_type == TokenType::ResolverDelegation {
                return Ok(AuthResolver { claims });
            }
        }

        // Fallback: legacy shared-secret `Resolver` push token.
        let claims = state.resolver.verify_token(&token)?;
        if claims.token_type != TokenType::Resolver {
            return Err(AppError::Unauthorized(
                "Invalid token type for resolver access".to_string(),
            ));
        }
        Ok(AuthResolver { claims })
    }
}
