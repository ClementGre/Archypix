use super::*;
use crate::infra::redis::{RedisKey, cache_get_json, cache_set_json_ex};
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::services::hierarchy::ResolvedDir;
use archypix_common::error::AppError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{trace, warn};
use uuid::Uuid;

/// A file a drop inbox received, listed for a short window so a client can verify the upload
/// (feature 39 §7). Listing-only: the picture itself is a real one.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentDrop {
    picture_id: Uuid,
    /// Unix seconds of the last write under this name.
    at: i64,
}

impl<'a> Vfs<'a> {
    fn drop_window(&self) -> i64 {
        self.state
            .settings
            .get(keys::WEBDAV_DROP_VERIFY_WINDOW_SECS) as i64
    }

    async fn recent_drops(&self, node_id: &str) -> Result<HashMap<String, RecentDrop>, AppError> {
        let window = self.drop_window();
        let now = Utc::now().timestamp();
        Ok(cache_get_json::<HashMap<String, RecentDrop>>(
            self.state.cache.as_ref(),
            RedisKey::WebdavRecentDrop(self.hierarchy_id, node_id),
        )
        .await?
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, d)| now - d.at < window)
        .collect())
    }

    async fn save_recent_drops(
        &self,
        node_id: &str,
        drops: &HashMap<String, RecentDrop>,
    ) -> Result<(), AppError> {
        let key = RedisKey::WebdavRecentDrop(self.hierarchy_id, node_id);
        if drops.is_empty() {
            return self.state.cache.del(key).await;
        }
        cache_set_json_ex(
            self.state.cache.as_ref(),
            key,
            drops,
            self.drop_window() as u64,
        )
        .await
    }

    /// Record that drop inbox `dir` received `name` (re-opening the window on a dedupe retry).
    /// Best-effort: the write itself already succeeded.
    pub(super) async fn note_drop(&self, dir: &ResolvedDir, name: &str, picture_id: Uuid) {
        let Some(node_id) = dir.node_id.as_deref() else {
            return;
        };
        if self.drop_window() <= 0 {
            return;
        }
        let res = async {
            let mut drops = self.recent_drops(node_id).await?;
            let at = Utc::now().timestamp();
            drops.insert(name.to_string(), RecentDrop { picture_id, at });
            self.save_recent_drops(node_id, &drops).await
        }
        .await;
        match res {
            Ok(()) => trace!(name, "vfs drop: listed for verification"),
            Err(e) => warn!(error = %e, "vfs drop: failed to record the verification entry"),
        }
    }

    /// Stop listing `name` in drop inbox `dir` (it was deleted or moved out). Best-effort.
    pub(super) async fn forget_drop(&self, dir: &ResolvedDir, name: &str) {
        let Some(node_id) = dir.node_id.as_deref() else {
            return;
        };
        let res = async {
            let mut drops = self.recent_drops(node_id).await?;
            if drops.remove(name).is_some() {
                self.save_recent_drops(node_id, &drops).await?;
            }
            Ok::<_, AppError>(())
        }
        .await;
        if let Err(e) = res {
            warn!(error = %e, "vfs drop: failed to drop the verification entry");
        }
    }

    /// The files drop inbox `dir` received within the window, under the names they were written as.
    pub(super) async fn drop_files(&self, dir: &ResolvedDir) -> Result<Vec<VfsEntry>, AppError> {
        let Some(node_id) = dir.node_id.as_deref() else {
            return Ok(vec![]);
        };
        let drops = self.recent_drops(node_id).await?;
        if drops.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<Uuid> = drops.values().map(|d| d.picture_id).collect();
        let pics: HashMap<Uuid, crate::domain::picture::Picture> =
            PictureRepository::list_by_ids(&self.state.db, &ids)
                .await?
                .into_iter()
                .filter(|p| p.local_user_id == self.user_id)
                .map(|p| (p.id, p))
                .collect();
        let mut out: Vec<VfsEntry> = drops
            .into_iter()
            .filter_map(|(name, d)| {
                pics.get(&d.picture_id)
                    .map(|p| picture_entry(p, name, true))
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
}
