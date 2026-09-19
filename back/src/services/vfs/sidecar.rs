
use crate::infra::redis::{RedisKey, cache_get_json, cache_set_json_ex};
use archypix_common::error::AppError;
use base64::Engine as _;
use chrono::Utc;
use std::collections::HashMap;
use tracing::trace;
use super::*;

impl<'a> Vfs<'a> {
    pub(super) async fn sidecar_map(&self, parent: &[String]) -> Result<HashMap<String, Sidecar>, AppError> {
        let key = path_key(parent);
        Ok(cache_get_json::<HashMap<String, Sidecar>>(
            self.state.cache.as_ref(),
            RedisKey::WebdavSidecar(self.hierarchy_id, &key),
        )
        .await?
        .unwrap_or_default())
    }

    /// All sidecar files stored under `parent`.
    pub(super) async fn sidecars(&self, parent: &[String]) -> Result<Vec<Sidecar>, AppError> {
        Ok(self.sidecar_map(parent).await?.into_values().collect())
    }

    /// The sidecar at `segments`, if any.
    pub(super) async fn sidecar(&self, segments: &[String]) -> Result<Option<Sidecar>, AppError> {
        let (parent, name) = split_last(segments)?;
        Ok(self.sidecar_map(parent).await?.remove(&name))
    }

    /// Store an OS-junk file as a sidecar so it round-trips in listings; oversized bodies are
    /// accepted but not stored (06_webdav.md §11).
    #[tracing::instrument(
        skip(self, bytes),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), bytes = bytes.len())
    )]
    pub async fn put_sidecar(
        &self,
        segments: &[String],
        bytes: &[u8],
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        let (parent, name) = split_last(segments)?;
        if bytes.len() > SIDECAR_MAX_BYTES {
            trace!("vfs sidecar: oversized — accepted without storing");
            return Ok(());
        }
        let key = path_key(parent);
        let mut map = self.sidecar_map(parent).await?;
        map.insert(
            name.clone(),
            Sidecar {
                name,
                size: bytes.len() as u64,
                mtime: Utc::now().timestamp(),
                content_type: content_type.map(|s| s.to_string()),
                data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        );
        cache_set_json_ex(
            self.state.cache.as_ref(),
            RedisKey::WebdavSidecar(self.hierarchy_id, &key),
            &map,
            TRANSIENT_TTL_SECS,
        )
        .await
    }

    /// Read a stored sidecar's bytes + content-type, if present.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn read_sidecar(
        &self,
        segments: &[String],
    ) -> Result<Option<(Vec<u8>, Option<String>)>, AppError> {
        let Some(sc) = self.sidecar(segments).await? else {
            return Ok(None);
        };
        let data = base64::engine::general_purpose::STANDARD
            .decode(sc.data_b64.as_bytes())
            .map_err(|e| AppError::InternalServerError(format!("decode sidecar: {e}")))?;
        Ok(Some((data, sc.content_type)))
    }

    /// Remove a stored sidecar (DELETE on an OS-junk file).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn delete_sidecar(&self, segments: &[String]) -> Result<(), AppError> {
        let (parent, name) = split_last(segments)?;
        let key = path_key(parent);
        let mut map = self.sidecar_map(parent).await?;
        if map.remove(&name).is_some() {
            let cache = self.state.cache.as_ref();
            let _ = if map.is_empty() {
                cache
                    .del(RedisKey::WebdavSidecar(self.hierarchy_id, &key))
                    .await
            } else {
                cache_set_json_ex(
                    cache,
                    RedisKey::WebdavSidecar(self.hierarchy_id, &key),
                    &map,
                    TRANSIENT_TTL_SECS,
                )
                .await
            };
        }
        Ok(())
    }

    // ── Atomic-save staging (08_webdav_issues.md §1) ──────────────────────────────

}
