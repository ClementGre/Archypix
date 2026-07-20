//! Shared JWT service, claims, and token taxonomy (feature 23 §9).
//!
//! Lifted out of the three hand-rolled copies (`back/src/infra/crypto.rs`,
//! `worker/src/auth.rs`, `resolver/src/handler.rs`) so signing/verification lives in one place.
//! Each consumer maps [`AuthError`] onto its own error type.

use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Token taxonomy. The wire form is lowercase for the original four; the delegation/session tokens
/// (feature 23 §3) use explicit snake_case names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
    User,
    Resolver,
    Federation,
    Worker,
    /// Backend-signed, `is_admin` delegation token the resolver replays on every backend call.
    #[serde(rename = "resolver_delegation")]
    ResolverDelegation,
    /// Resolver-signed operator session token for the fleet admin dashboard.
    #[serde(rename = "resolver_admin_session")]
    ResolverAdminSession,
    /// Owner-backend-signed session for an unlocked password-gated public share (feature 27 §6).
    /// The `sub` claim carries the `public_shares.id`; short TTL.
    #[serde(rename = "public_share")]
    PublicShare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject: username (user tokens), global domain (federation), worker_id (worker),
    /// or `"resolver"` (delegation).
    pub sub: String,
    /// User UUID — present for user tokens, absent otherwise.
    pub uid: Option<Uuid>,
    pub is_admin: bool,
    /// Global (federation) domain of the issuing instance (e.g. `example.com`).
    pub instance: String,
    pub token_type: TokenType,
    /// Audience: backend domain of the verifying instance (matched against `BACK_DOMAIN` on decode).
    pub aud: String,
    /// Issuer: backend domain of the signing instance.
    pub iss: String,
    /// Expiry timestamp (Unix seconds).
    pub exp: i64,
    /// Issued-at timestamp (Unix seconds).
    pub iat: i64,
    /// Unique token ID — used for replay protection.
    pub jti: String,
}

/// JWT failure, split so consumers can map encoding (internal, 500) apart from verification
/// (client, 401).
#[derive(Debug)]
pub enum AuthError {
    Encode(jsonwebtoken::errors::Error),
    Verify(jsonwebtoken::errors::Error),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::Encode(e) => write!(f, "token encoding failed: {e}"),
            AuthError::Verify(e) => write!(f, "token verification failed: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Stateful HS256 JWT service — held in `AppState`, shared across requests.
#[derive(Clone)]
pub struct JwtService {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    issuer: String,
}

impl JwtService {
    pub fn new(secret: &str, issuer: &str) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            issuer: issuer.to_string(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &self,
        subject: &str,
        uid: Option<Uuid>,
        instance: &str,
        token_type: TokenType,
        is_admin: bool,
        audience: &str,
        ttl_secs: i64,
    ) -> Result<String, AuthError> {
        let now = Utc::now().timestamp();
        let claims = JwtClaims {
            sub: subject.to_string(),
            uid,
            instance: instance.to_string(),
            token_type,
            is_admin,
            aud: audience.to_string(),
            iss: self.issuer.clone(),
            exp: now + ttl_secs,
            iat: now,
            jti: Uuid::new_v4().to_string(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(AuthError::Encode)
    }

    /// Decode + verify, checking both audience and this service's issuer.
    pub fn decode(&self, token: &str, audience: &str) -> Result<JwtClaims, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[audience]);
        validation.set_issuer(&[self.issuer.clone()]);
        decode::<JwtClaims>(token, &self.decoding_key, &validation)
            .map(|d| d.claims)
            .map_err(AuthError::Verify)
    }

    /// Decode + verify audience only — used for tokens signed by a peer whose issuer varies
    /// (workers on any host, resolver-issued tokens).
    pub fn decode_any_issuer(&self, token: &str, audience: &str) -> Result<JwtClaims, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[audience]);
        decode::<JwtClaims>(token, &self.decoding_key, &validation)
            .map(|d| d.claims)
            .map_err(AuthError::Verify)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_decode_roundtrips() {
        let svc = JwtService::new("a-secret-of-reasonable-length", "backend.example.com");
        let token = svc
            .issue(
                "alice",
                None,
                "example.com",
                TokenType::User,
                false,
                "backend.example.com",
                300,
            )
            .unwrap();
        let claims = svc.decode(&token, "backend.example.com").unwrap();
        assert_eq!(claims.sub, "alice");
        assert_eq!(claims.token_type, TokenType::User);
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let svc = JwtService::new("a-secret-of-reasonable-length", "backend.example.com");
        let token = svc
            .issue("a", None, "e", TokenType::Worker, false, "aud-a", 300)
            .unwrap();
        assert!(svc.decode(&token, "aud-b").is_err());
    }

    #[test]
    fn wrong_issuer_rejected_but_any_issuer_accepts() {
        let signer = JwtService::new("shared", "worker-7");
        let verifier = JwtService::new("shared", "backend.example.com");
        let token = signer
            .issue(
                "worker-7",
                None,
                "e",
                TokenType::Worker,
                false,
                "backend.example.com",
                300,
            )
            .unwrap();
        assert!(verifier.decode(&token, "backend.example.com").is_err());
        assert!(
            verifier
                .decode_any_issuer(&token, "backend.example.com")
                .is_ok()
        );
    }

    #[test]
    fn delegation_token_type_serializes_snake_case() {
        let json = serde_json::to_string(&TokenType::ResolverDelegation).unwrap();
        assert_eq!(json, "\"resolver_delegation\"");
    }
}
