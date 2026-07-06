//! Auth extractors, mirroring the backend's `api/middleware`.
//!
//! - [`AuthPush`] — a backend→resolver **push** token (shared-secret `Resolver`), for self-register /
//!   mapping-update / heartbeat.
//! - [`AuthAdmin`] — an operator `ResolverAdminSession` token, for the fleet dashboard endpoints.

use crate::state::AppState;
use archypix_common::auth::{JwtClaims, TokenType};
use archypix_common::error::AppError;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

pub fn bearer_token(parts: &Parts) -> Result<&str, AppError> {
    parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            AppError::Unauthorized("Missing or invalid Authorization header".to_string())
        })
}

fn decode(state: &AppState, token: &str, expected: TokenType) -> Result<JwtClaims, AppError> {
    let claims = state
        .jwt
        .decode_any_issuer(token, &state.global_domain())
        .map_err(|e| AppError::Unauthorized(e.to_string()))?;
    if claims.token_type != expected {
        return Err(AppError::Unauthorized("Invalid token type".to_string()));
    }
    Ok(claims)
}

/// A backend's shared-secret push token.
pub struct AuthPush(pub JwtClaims);

impl FromRequestParts<AppState> for AuthPush {
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)?;
        Ok(AuthPush(decode(state, token, TokenType::Resolver)?))
    }
}

/// An operator dashboard session token.
pub struct AuthAdmin(pub JwtClaims);

impl FromRequestParts<AppState> for AuthAdmin {
    type Rejection = AppError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)?;
        Ok(AuthAdmin(decode(
            state,
            token,
            TokenType::ResolverAdminSession,
        )?))
    }
}
