use crate::error::{Result, WorkerError};
use archypix_common::auth::{JwtService, TokenType};

/// Generate a fresh worker JWT valid for 300 seconds.
///
/// `aud` must be the backend domain this token is intended for. Issuer + subject are the `worker_id`
/// (the backend verifies worker tokens with `decode_any_issuer`, so the worker may run on any host).
pub fn generate_token(
    worker_id: &str,
    global_domain: &str,
    back_domain: &str,
    worker_jwt_secret: &str,
) -> Result<String> {
    JwtService::new(worker_jwt_secret, worker_id)
        .issue(
            worker_id,
            None,
            global_domain,
            TokenType::Worker,
            false,
            back_domain,
            300,
        )
        .map_err(|e| WorkerError::Jwt(e.to_string()))
}
