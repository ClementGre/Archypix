use crate::api::middleware::auth_user::AuthUser;
use crate::domain::auth::JwtClaims;
use crate::infra::error::AppError;
use crate::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

#[derive(Clone, Debug)]
pub struct AuthAdmin {
    pub claims: JwtClaims,
}

impl FromRequestParts<AppState> for AuthAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_user = AuthUser::from_request_parts(parts, state).await?;

        if !auth_user.claims.is_admin {
            return Err(AppError::Unauthorized("Admin access required".to_string()));
        }

        Ok(AuthAdmin {
            claims: auth_user.claims,
        })
    }
}
