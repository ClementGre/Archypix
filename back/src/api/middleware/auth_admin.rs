use super::bearer_token;
use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::settings::keys;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// Admin guard with **two accepted issuers** (feature 23 §3.3):
/// - a normal **user** token with `is_admin` (direct login on `/admin`), or
/// - a backend-signed **`ResolverDelegation`** token (the resolver admin proxy), which is `is_admin`
///   and attributed to `sub = "resolver"`.
///
/// Both are signed with this backend's `JWT_SECRET` (`state.jwt`), so one decode covers both.
#[derive(Clone, Debug)]
pub struct AuthAdmin {
    pub claims: JwtClaims,
}

impl AuthAdmin {
    /// True when the request came through the resolver proxy rather than a direct user login.
    pub fn is_delegated(&self) -> bool {
        self.claims.token_type == TokenType::ResolverDelegation
    }
}

impl FromRequestParts<AppState> for AuthAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        let claims = state
            .jwt
            .decode(&token, &state.settings.get(keys::BACK_DOMAIN))?;

        let accepted = match claims.token_type {
            TokenType::User | TokenType::ResolverDelegation => claims.is_admin,
            _ => false,
        };
        if !accepted {
            return Err(AppError::Unauthorized("Admin access required".to_string()));
        }

        if let Some(uid) = &claims.uid {
            tracing::Span::current().record("enduser.id", tracing::field::display(uid));
        }

        Ok(AuthAdmin { claims })
    }
}
