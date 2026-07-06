//! Shared HTTP error type (feature 23 §8/9 follow-up), used by `back` and `resolver` so the two
//! near-identical hand-rolled `AppError` enums don't drift. Conversions from crate-specific error
//! types (`AuthError`, `SettingsError`, `sqlx::Error`, `anyhow::Error`) live here too, since orphan
//! rules forbid implementing a foreign trait (`From`) for a foreign type once `AppError` itself
//! moves out of `back`/`resolver`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use thiserror::Error;
use tracing::{error, warn};

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Not found")]
    NotFound,
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Internal server error: {0}")]
    InternalServerError(String),
    #[error("Database error")]
    DatabaseError(String, String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Method not allowed: {0}")]
    MethodNotAllowed(String),
    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),
    #[error("Insufficient storage: {0}")]
    InsufficientStorage(String),
    #[error("Too many requests: {0}")]
    TooManyRequests(String),
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
    /// Propagated HTTP error from a proxied backend (status code + body) — resolver's per-instance
    /// admin proxy (feature 23 §5.3).
    #[error("Backend error {0}: {1}")]
    BackendError(u16, String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::InternalServerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::DatabaseError(_, _) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::MethodNotAllowed(_) => StatusCode::METHOD_NOT_ALLOWED,
            AppError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            AppError::InsufficientStorage(_) => StatusCode::INSUFFICIENT_STORAGE,
            AppError::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            AppError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            AppError::BackendError(code, _) => {
                StatusCode::from_u16(*code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            }
        };
        let message = match &self {
            // Proxied backend errors already carry a client-safe body; everything else renders
            // via `Display` (which is generic enough not to leak internals for the 5xx variants).
            AppError::BackendError(_, msg) => msg.clone(),
            _ => self.to_string(),
        };
        let body = serde_json::json!({ "error": message });
        if status.is_server_error() {
            error!(status = status.as_u16(), error = ?self, "server error");
        } else {
            warn!(status = status.as_u16(), error = ?self, "client error");
        }
        (status, axum::Json(body)).into_response()
    }
}

#[cfg(feature = "sqlx")]
pub fn map_sqlx_error(err: sqlx::Error) -> AppError {
    use std::borrow::Cow;
    if let sqlx::Error::Database(_) = &err {
        let db_error = err.into_database_error().unwrap();
        if let Some(Cow::Borrowed("23505")) = db_error.code() {
            return AppError::Conflict(db_error.message().to_string());
        }
        AppError::DatabaseError(
            db_error.code().unwrap_or_default().to_string(),
            db_error.message().to_string(),
        )
    } else {
        AppError::InternalServerError(err.to_string())
    }
}

/// SQLx errors from a runtime query layer (e.g. resolver's `?`-based `repository.rs`) map to an
/// internal error. Callers that need the sharper unique-violation → `Conflict` mapping use
/// `map_sqlx_error` explicitly instead of `?`.
#[cfg(feature = "sqlx")]
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::InternalServerError(err.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::InternalServerError(err.to_string())
    }
}

#[cfg(feature = "auth")]
impl From<crate::auth::AuthError> for AppError {
    fn from(e: crate::auth::AuthError) -> Self {
        match e {
            crate::auth::AuthError::Encode(err) => AppError::InternalServerError(err.to_string()),
            crate::auth::AuthError::Verify(err) => AppError::Unauthorized(err.to_string()),
        }
    }
}

/// Map a settings-engine error onto the API error type (env-locked → 409, else 400).
#[cfg(feature = "settings")]
impl From<crate::settings::SettingsError> for AppError {
    fn from(e: crate::settings::SettingsError) -> Self {
        match e {
            crate::settings::SettingsError::Locked(_) => AppError::Conflict(e.to_string()),
            _ => AppError::BadRequest(e.to_string()),
        }
    }
}
