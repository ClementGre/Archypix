use super::bearer_token;
use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::error::AppError;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

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
        if !state.config.use_resolver {
            return Err(AppError::Unauthorized(
                "Resolver is disabled on this backend. Set USE_RESOLVER=true and RESOLVER_JWT_SECRET to enable it.".to_string(),
            ));
        }

        let token = bearer_token(&parts.headers)?;
        let claims = state.resolver.verify_token(&token)?;

        if claims.token_type != TokenType::Resolver {
            return Err(AppError::Unauthorized(
                "Invalid token type for resolver access".to_string(),
            ));
        }

        Ok(AuthResolver { claims })
    }
}
