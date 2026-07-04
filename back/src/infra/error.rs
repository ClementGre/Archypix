use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sqlx::Error;
use std::borrow::Cow;
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
        };
        let body = serde_json::json!({ "error": self.to_string() });
        if status.is_server_error() {
            error!(status = status.as_u16(), error = ?self, "server error");
        } else {
            warn!(status = status.as_u16(), error = ?self, "client error");
        }
        (status, axum::Json(body)).into_response()
    }
}

pub fn map_sqlx_error(err: sqlx::Error) -> AppError {
    if let Error::Database(_) = &err {
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
