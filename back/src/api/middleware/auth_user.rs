use super::bearer_token;
use crate::domain::auth::{JwtClaims, TokenType};
use crate::infra::settings::keys;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct AuthUser {
    pub claims: JwtClaims,
}

impl AuthUser {
    pub fn user_id(&self) -> Result<Uuid, AppError> {
        self.claims
            .uid
            .ok_or_else(|| AppError::Unauthorized("Missing user id in token".to_string()))
    }

    /// Returns `"admin"` or `"user"` depending on the `is_admin` claim — used as a consistent
    /// `token_type` field in structured log records.
    pub fn token_type(&self) -> &'static str {
        if self.claims.is_admin {
            "admin"
        } else {
            "user"
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        let claims = state
            .jwt
            .decode(&token, &state.settings.get(keys::BACK_DOMAIN))?;

        if claims.token_type != TokenType::User {
            return Err(AppError::Unauthorized("Invalid token type".to_string()));
        }
        if claims.uid.is_none() {
            return Err(AppError::Unauthorized("Missing user id".to_string()));
        }

        if let Some(uid) = &claims.uid {
            // `enduser.id` is the OTel semantic-convention field reserved on the `http_request` span.
            tracing::Span::current().record("enduser.id", tracing::field::display(uid));
        }

        Ok(AuthUser { claims })
    }
}
