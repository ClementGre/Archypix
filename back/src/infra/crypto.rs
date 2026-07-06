use archypix_common::error::AppError;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use rand::Rng;
use sha2::{Digest, Sha256};

// `JwtService` + claims live in `archypix_common::auth` (feature 23 §9). Re-exported so existing
// `crate::infra::crypto::JwtService` imports keep working. `AuthError` maps to `AppError` below so
// call sites keep their `?`-into-`AppError` ergonomics (encode → 500, verify → 401).
pub use archypix_common::auth::{AuthError, JwtService};

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = argon2::password_hash::SaltString::generate(argon2::password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::InternalServerError(e.to_string()))
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed =
        PasswordHash::new(hash).map_err(|e| AppError::InternalServerError(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Perform an Argon2 verification against a fixed dummy hash and discard the result.
///
/// Called on the login path when the username does not exist (or has no stored credential) so the
/// response latency matches the credential-present path — closing the user-enumeration timing
/// side-channel. The dummy hash is computed once on first use.
pub fn verify_password_dummy(password: &str) {
    use std::sync::OnceLock;
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    let hash = DUMMY_HASH.get_or_init(|| {
        hash_password("archypix-dummy-password-for-timing-equalization")
            .unwrap_or_else(|_| String::new())
    });
    if !hash.is_empty() {
        let _ = verify_password(password, hash);
    }
}

pub fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn hash_refresh_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

// ── WebDAV per-hierarchy token (encrypted at rest) ──────────────────────────────
//
// A WebDAV token is a 32-byte random value (hex). Because it must be displayable at any
// time (the owner pastes it into a client), it is stored **encrypted** with AES-256-GCM
// rather than hashed. The key is a SHA-256 domain-separated sub-key of `JWT_SECRET` — no
// new env var, and the JWT HMAC key is never used directly as a cipher key. The on-disk
// blob is `nonce(12) ‖ ciphertext ‖ tag(16)`. See doc/features/06_webdav.md §3.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};

const WEBDAV_KEY_LABEL: &[u8] = b"archypix-webdav-token-enc-v1";

/// Generate a fresh WebDAV token (64-char hex of 32 random bytes).
pub fn generate_webdav_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn webdav_cipher(jwt_secret: &str) -> Aes256Gcm {
    let mut hasher = Sha256::new();
    hasher.update(WEBDAV_KEY_LABEL);
    hasher.update(jwt_secret.as_bytes());
    let key_bytes = hasher.finalize();
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    Aes256Gcm::new(key)
}

/// Encrypt a WebDAV token for storage. Returns `nonce ‖ ciphertext ‖ tag`.
pub fn encrypt_webdav_token(jwt_secret: &str, token: &str) -> Result<Vec<u8>, AppError> {
    let cipher = webdav_cipher(jwt_secret);
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, token.as_bytes())
        .map_err(|_| AppError::InternalServerError("webdav token encryption failed".into()))?;
    let mut blob = Vec::with_capacity(12 + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

/// Decrypt a stored WebDAV token blob (`nonce ‖ ciphertext ‖ tag`).
pub fn decrypt_webdav_token(jwt_secret: &str, blob: &[u8]) -> Result<String, AppError> {
    if blob.len() < 12 + 16 {
        return Err(AppError::InternalServerError(
            "webdav token blob too short".into(),
        ));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let cipher = webdav_cipher(jwt_secret);
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| AppError::InternalServerError("webdav token decryption failed".into()))?;
    String::from_utf8(plaintext)
        .map_err(|_| AppError::InternalServerError("webdav token not utf-8".into()))
}

/// SHA-256 hex of a WebDAV token — used as the Redis cache key so the plaintext token
/// never becomes a key.
pub fn hash_webdav_token(token: &str) -> String {
    hash_refresh_token(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webdav_token_roundtrips() {
        let secret = "a-test-jwt-secret-of-reasonable-length";
        let token = generate_webdav_token();
        let blob = encrypt_webdav_token(secret, &token).unwrap();
        assert_ne!(blob, token.as_bytes());
        let back = decrypt_webdav_token(secret, &blob).unwrap();
        assert_eq!(back, token);
    }

    #[test]
    fn webdav_token_wrong_secret_fails() {
        let token = generate_webdav_token();
        let blob = encrypt_webdav_token("secret-one", &token).unwrap();
        assert!(decrypt_webdav_token("secret-two", &blob).is_err());
    }

    #[test]
    fn webdav_token_tamper_fails() {
        let secret = "a-test-jwt-secret";
        let token = generate_webdav_token();
        let mut blob = encrypt_webdav_token(secret, &token).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xff;
        assert!(decrypt_webdav_token(secret, &blob).is_err());
    }
}
