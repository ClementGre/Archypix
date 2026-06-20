//! Physical-purge sweep for soft-deleted owned pictures (09 §5.1).
//!
//! A [`RecurringTask`] that finds owned pictures whose retention window has elapsed
//! (`deleted_at + user_settings.trash_retention_days < now`, derived per-owner so a retention change
//! needs no backfill), unannounces them from any share recipients, deletes their S3 objects
//! (original + thumbnails + versions), and hard-deletes the row. Removing the row drops its tags
//! (coverage), so the unannounce diff would also reach recipients eventually — but `share_announcements`
//! has no FK to `pictures`, so this task explicitly unannounces and deletes the tracking rows itself,
//! mirroring the revocation cascade (`cleanup_incoming_share`).

use crate::infra::config::Config;
use crate::infra::error::AppError;
use crate::infra::redis::{Cache, RedisKey};
use crate::infra::s3::{self, Storage};
use crate::infra::scheduler::RecurringTask;
use crate::infra::tasks::{InternalTask, TaskQueue};
use crate::repository::picture::PictureRepository;
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::share_announcement::ShareAnnouncementRepository;
use crate::repository::user::UserRepository;
use crate::services::users::find_local_user_id;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

/// Periodically purges owned, retention-expired soft-deleted pictures.
pub struct PurgeSweepTask {
    db: PgPool,
    storage: Arc<dyn Storage>,
    cache: Arc<dyn Cache>,
    config: Config,
    task_queue: TaskQueue,
    interval: Duration,
    batch: i64,
}

impl PurgeSweepTask {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: PgPool,
        storage: Arc<dyn Storage>,
        cache: Arc<dyn Cache>,
        config: Config,
        task_queue: TaskQueue,
        interval: Duration,
        batch: i64,
    ) -> Self {
        Self {
            db,
            storage,
            cache,
            config,
            task_queue,
            interval,
            batch,
        }
    }

    /// Purge one picture: unannounce + delete tracking rows in a transaction, then best-effort S3
    /// cleanup and presign-cache invalidation.
    async fn purge_one(&self, picture_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
        // Versions to clean from S3 (read before the row is deleted).
        let versions = PictureVersionRepository::list_by_picture(&self.db, picture_id).await?;
        let owner_username = UserRepository::find_by_id(&self.db, user_id)
            .await?
            .map(|u| u.username)
            .unwrap_or_default();

        // ── Tracking teardown (downstream gathered before tracking rows are deleted) ──
        let mut tx = self
            .db
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        let downstream =
            ShareAnnouncementRepository::find_downstream_for_pictures(&mut *tx, &[picture_id])
                .await?;
        ShareAnnouncementRepository::delete_for_pictures(&mut *tx, &[picture_id]).await?;
        PictureRepository::hard_delete(&mut *tx, picture_id).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        // ── Unannounce to recipients, grouped per outgoing share (best-effort) ──
        let mut by_share: HashMap<Uuid, (String, String, Vec<String>)> = HashMap::new();
        for d in downstream {
            let entry = by_share.entry(d.outgoing_share_id).or_insert_with(|| {
                (
                    d.recipient_username.clone(),
                    d.recipient_instance.clone(),
                    vec![],
                )
            });
            entry.2.push(d.announce_id);
        }
        for (os_id, (recipient_username, recipient_instance, picture_ids)) in by_share {
            let is_same_backend = find_local_user_id(
                self.cache.as_ref(),
                &self.db,
                &self.config,
                &recipient_username,
                &recipient_instance,
            )
            .await?
            .is_some();
            self.task_queue
                .enqueue(InternalTask::UnannounceSharedPictures {
                    outgoing_share_id: os_id,
                    sender_username: owner_username.clone(),
                    recipient_username,
                    recipient_instance,
                    picture_ids,
                    is_same_backend,
                });
        }

        // ── S3 cleanup (best-effort: a missing object is not an error) ──
        let key = s3::picture_key(user_id, picture_id);
        for bucket in [
            &self.config.s3_bucket_pictures,
            &self.config.s3_bucket_small,
            &self.config.s3_bucket_medium,
            &self.config.s3_bucket_large,
        ] {
            if let Err(e) = self.storage.delete_object(bucket, &key).await {
                warn!(picture_id = %picture_id, bucket, error = ?e, "purge: failed to delete S3 object");
            }
        }
        for v in &versions {
            let vkey = s3::version_key(user_id, picture_id, v.id);
            if let Err(e) = self
                .storage
                .delete_object(&self.config.s3_bucket_versions, &vkey)
                .await
            {
                warn!(picture_id = %picture_id, version = %v.id, error = ?e, "purge: failed to delete version object");
            }
        }

        // Invalidate any cached presigned URLs (now pointing at deleted objects).
        for variant in ["original", "small", "medium", "large"] {
            let _ = self
                .cache
                .del(RedisKey::PictureUrl(picture_id, variant))
                .await;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl RecurringTask for PurgeSweepTask {
    fn name(&self) -> &'static str {
        "purge_sweep"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    async fn tick(&self) -> anyhow::Result<()> {
        let purgeable = PictureRepository::find_purgeable(&self.db, self.batch).await?;
        if purgeable.is_empty() {
            return Ok(());
        }
        let mut purged = 0usize;
        for (picture_id, user_id) in purgeable {
            match self.purge_one(picture_id, user_id).await {
                Ok(()) => purged += 1,
                Err(e) => {
                    warn!(picture_id = %picture_id, error = ?e, "purge: failed to purge picture")
                }
            }
        }
        if purged > 0 {
            info!(
                purged,
                "purge sweep: physically purged retention-expired pictures"
            );
        }
        Ok(())
    }
}
