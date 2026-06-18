use crate::error::{Result, WorkerError};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// JWT claims structure matching the backend's `JwtClaims`.
#[derive(Debug, Serialize, Deserialize)]
struct WorkerClaims {
    sub: String,
    uid: Option<Uuid>,
    is_admin: bool,
    instance: String,
    token_type: String,
    aud: String,
    iss: String,
    exp: i64,
    iat: i64,
    jti: String,
}

/// Generate a fresh worker JWT valid for 300 seconds.
///
/// `aud` must be the backend domain this token is intended for.
pub fn generate_token(
    worker_id: &str,
    global_domain: &str,
    back_domain: &str,
    worker_jwt_secret: &str,
) -> Result<String> {
    let now = Utc::now().timestamp();
    let claims = WorkerClaims {
        sub: worker_id.to_string(),
        uid: None,
        is_admin: false,
        instance: global_domain.to_string(),
        token_type: "worker".to_string(),
        aud: back_domain.to_string(),
        iss: worker_id.to_string(),
        exp: now + 300,
        iat: now,
        jti: Uuid::new_v4().to_string(),
    };
    let key = EncodingKey::from_secret(worker_jwt_secret.as_bytes());
    encode(&Header::new(Algorithm::HS256), &claims, &key)
        .map_err(|e| WorkerError::Jwt(e.to_string()))
}
