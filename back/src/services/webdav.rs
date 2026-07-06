//! WebDAV Basic-auth resolution (06_webdav.md §3.3).
//!
//! A WebDAV client presents `Authorization: Basic base64(username:token)`. The token is the
//! per-hierarchy secret; it alone is authoritative. We resolve it to a session
//! `(user_id, hierarchy_id, use_redirect)`, caching by the token's SHA-256 so the plaintext is
//! never a Redis key.

use crate::infra::crypto;
use crate::infra::redis::{cache_get_json, cache_set_json_ex, RedisKey};
use crate::infra::settings::keys;
use crate::repository::hierarchy::HierarchyRepository;
use crate::repository::user::UserRepository;
use crate::state::AppState;
use archypix_common::error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SESSION_TTL_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavSession {
    pub user_id: Uuid,
    pub hierarchy_id: Uuid,
    pub use_redirect: bool,
}

/// Resolve a Basic-auth `(username, token)` pair and the mount `slug` to a session. Fails
/// closed (`Unauthorized`) on any mismatch or when the hierarchy is disabled.
#[tracing::instrument(skip(state, token))]
pub async fn authenticate(
    state: &AppState,
    username: &str,
    token: &str,
    slug: &str,
) -> Result<WebdavSession, AppError> {
    let token_hash = crypto::hash_webdav_token(token);

    if let Some(session) =
        cache_get_json::<WebdavSession>(state.cache.as_ref(), RedisKey::WebdavToken(&token_hash))
            .await?
    {
        return Ok(session);
    }

    let local = local_username(username);
    let user = UserRepository::find_by_username(&state.db, &local)
        .await?
        .ok_or_else(|| AppError::Unauthorized("unknown user".into()))?;

    // Candidates: the user's hierarchies whose name slugifies to the mount slug (usually one).
    let hierarchies = HierarchyRepository::list_by_owner(&state.db, user.id).await?;
    for h in hierarchies {
        if crate::domain::hierarchy::slugify(&h.name) != slug {
            continue;
        }
        let Some(row) = HierarchyRepository::get_webdav(&state.db, user.id, h.id).await? else {
            continue;
        };
        if !row.enabled {
            continue;
        }
        let Some(blob) = row.webdav_token_enc else {
            continue;
        };
        let stored = crypto::decrypt_webdav_token(&state.settings.get(keys::JWT_SECRET), &blob)?;
        if constant_time_eq(stored.as_bytes(), token.as_bytes()) {
            let session = WebdavSession {
                user_id: user.id,
                hierarchy_id: h.id,
                use_redirect: row.webdav_use_redirect,
            };
            let _ = cache_set_json_ex(
                state.cache.as_ref(),
                RedisKey::WebdavToken(&token_hash),
                &session,
                SESSION_TTL_SECS,
            )
            .await;
            return Ok(session);
        }
    }

    Err(AppError::Unauthorized("invalid WebDAV credentials".into()))
}

/// Extract the local username from a Basic-auth username field, tolerating `@user`,
/// `user@domain`, and `user:instance` forms.
fn local_username(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('@');
    trimmed
        .split(['@', ':'])
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_username_forms() {
        assert_eq!(local_username("@alice"), "alice");
        assert_eq!(local_username("alice@example.com"), "alice");
        assert_eq!(local_username("alice:example.com"), "alice");
        assert_eq!(local_username("alice"), "alice");
    }

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
