
use crate::infra::redis::{RedisKey, cache_get_json, cache_set_json_ex};
use crate::infra::settings::keys;
use archypix_common::error::AppError;
use chrono::Utc;
use std::path::Path;
use tracing::trace;
use uuid::Uuid;
use super::*;

impl<'a> Vfs<'a> {
    /// The scratch namespace recorded under `parent`.
    pub(super) async fn staging_parent(&self, parent: &[String]) -> Result<StagingParent, AppError> {
        let key = path_key(parent);
        Ok(cache_get_json::<StagingParent>(
            self.state.cache.as_ref(),
            RedisKey::WebdavStaging(self.hierarchy_id, &key),
        )
        .await?
        .unwrap_or_default())
    }

    /// Persist (or drop, when empty) the scratch namespace under `parent`.
    pub(super) async fn save_staging_parent(
        &self,
        parent: &[String],
        sp: &StagingParent,
    ) -> Result<(), AppError> {
        let key = path_key(parent);
        let cache = self.state.cache.as_ref();
        if sp.dirs.is_empty() && sp.files.is_empty() {
            let _ = cache
                .del(RedisKey::WebdavStaging(self.hierarchy_id, &key))
                .await;
            Ok(())
        } else {
            cache_set_json_ex(
                cache,
                RedisKey::WebdavStaging(self.hierarchy_id, &key),
                sp,
                TRANSIENT_TTL_SECS,
            )
            .await
        }
    }

    /// Whether `segments` names a staged (MKCOL'd) scratch directory.
    pub(super) async fn is_staging_dir(&self, segments: &[String]) -> Result<bool, AppError> {
        let Some((name, parent)) = segments.split_last() else {
            return Ok(false);
        };
        Ok(self
            .staging_parent(parent)
            .await?
            .dirs
            .iter()
            .any(|d| d == name))
    }

    /// The staged file at `segments`, if any.
    pub(super) async fn staged_file(&self, segments: &[String]) -> Result<Option<StagedFile>, AppError> {
        let (parent, name) = split_last(segments)?;
        Ok(self.staging_parent(parent).await?.files.remove(&name))
    }

    /// `MKCOL` of a scratch temp directory: record it so it round-trips under its exact name until
    /// a file lands and a rename promotes it (never mints a tag, unlike a mirror pending dir §9).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn add_staging_dir(&self, segments: &[String]) -> Result<(), AppError> {
        let (parent, name) = split_last(segments)?;
        let mut sp = self.staging_parent(parent).await?;
        if !sp.dirs.iter().any(|d| d == &name) {
            trace!("vfs staging: recorded scratch directory");
            sp.dirs.push(name);
        }
        self.save_staging_parent(parent, &sp).await
    }

    /// `PUT` of scratch bytes: stream them to the staging bucket and record a marker (never a
    /// picture). A terminal MOVE promotes them (§1.6).
    #[tracing::instrument(
        skip(self, temp_path),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), hash = %hash, bytes = size)
    )]
    pub async fn put_staging(
        &self,
        segments: &[String],
        temp_path: &Path,
        hash: &str,
        size: i64,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        let (parent, name) = split_last(segments)?;
        let staging_key = format!("webdav/{}/{}", self.hierarchy_id, Uuid::new_v4());
        self.state
            .storage
            .put_object_file(
                &self.state.settings.get(keys::S3_BUCKET_STAGING),
                &staging_key,
                temp_path,
                content_type,
            )
            .await?;
        let mut sp = self.staging_parent(parent).await?;
        // Replacing an earlier staged version — drop its now-orphaned object.
        if let Some(old) = sp.files.insert(
            name.clone(),
            StagedFile {
                name,
                size,
                mtime: Utc::now().timestamp(),
                hash: Some(hash.to_string()),
                content_type: content_type.map(|s| s.to_string()),
                staging_key: Some(staging_key),
                picture_ref: None,
            },
        ) {
            if let Some(key) = old.staging_key {
                let _ = self
                    .state
                    .storage
                    .delete_object(&self.state.settings.get(keys::S3_BUCKET_STAGING), &key)
                    .await;
            }
        }
        trace!("vfs staging: stored scratch bytes");
        self.save_staging_parent(parent, &sp).await
    }

    /// Record a backup reference to an existing picture when a client MOVEs/COPYs a real file into a
    /// scratch path (the "move the original out of the way" step, §1.5). Mutates no picture.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, from = %from_real.join("/"), to = %to_staging.join("/")
    ))]
    pub async fn stage_backup_ref(
        &self,
        from_real: &[String],
        to_staging: &[String],
    ) -> Result<(), AppError> {
        // If the source isn't a real file, there is nothing to reference — accept silently.
        let Ok(entry) = self.file_entry(from_real).await else {
            return Ok(());
        };
        let Some(pid) = entry.picture_id else {
            return Ok(());
        };
        let (parent, name) = split_last(to_staging)?;
        let mut sp = self.staging_parent(parent).await?;
        sp.files.insert(
            name.clone(),
            StagedFile {
                name,
                size: entry.size as i64,
                mtime: Utc::now().timestamp(),
                hash: entry.etag,
                content_type: entry.mime_type,
                staging_key: None,
                picture_ref: Some(pid),
            },
        );
        trace!("vfs staging: recorded backup reference");
        self.save_staging_parent(parent, &sp).await
    }

    /// `GET`/`HEAD` on a scratch path — serve staged bytes (from the staging bucket) or the
    /// referenced picture for a backup reference.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn read_staging(&self, segments: &[String]) -> Result<Option<ReadTarget>, AppError> {
        let Some(f) = self.staged_file(segments).await? else {
            return Ok(None);
        };
        if let Some(pid) = f.picture_ref {
            return Ok(Some(self.read_picture(pid).await?));
        }
        let Some(key) = f.staging_key else {
            return Ok(None);
        };
        if self.use_redirect {
            let url = self
                .state
                .storage
                .presign_get(&self.state.settings.get(keys::S3_BUCKET_STAGING), &key)
                .await?;
            Ok(Some(ReadTarget::Redirect(url)))
        } else {
            let data = self
                .state
                .storage
                .get_object(&self.state.settings.get(keys::S3_BUCKET_STAGING), &key)
                .await?;
            Ok(Some(ReadTarget::Bytes {
                data,
                mime: f.content_type,
            }))
        }
    }

    /// `DELETE` on a scratch path: drop the file marker (and its staged object) or the temp
    /// directory (and everything staged under it). No picture is touched.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn delete_staging(&self, segments: &[String]) -> Result<(), AppError> {
        let (parent, name) = split_last(segments)?;
        let mut sp = self.staging_parent(parent).await?;
        if let Some(f) = sp.files.remove(&name) {
            if let Some(key) = f.staging_key {
                let _ = self
                    .state
                    .storage
                    .delete_object(&self.state.settings.get(keys::S3_BUCKET_STAGING), &key)
                    .await;
            }
            trace!("vfs staging: dropped scratch file");
            return self.save_staging_parent(parent, &sp).await;
        }
        if let Some(pos) = sp.dirs.iter().position(|d| d == &name) {
            sp.dirs.remove(pos);
            self.save_staging_parent(parent, &sp).await?;
            // Sweep any files staged inside the removed directory.
            let inner = self.staging_parent(segments).await?;
            for f in inner.files.values() {
                if let Some(key) = &f.staging_key {
                    let _ = self
                        .state
                        .storage
                        .delete_object(&self.state.settings.get(keys::S3_BUCKET_STAGING), key)
                        .await;
                }
            }
            let _ = self
                .state
                .cache
                .del(RedisKey::WebdavStaging(
                    self.hierarchy_id,
                    &path_key(segments),
                ))
                .await;
            trace!("vfs staging: dropped scratch directory");
        }
        Ok(())
    }

    /// Promote staged bytes to a real picture (§1.6): the terminal MOVE/COPY of an atomic save.
    /// `remove_source` clears the scratch marker (MOVE) or keeps it (COPY). Returns whether a new
    /// resource was created at `to`.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, from = %from.join("/"), to = %to.join("/")))]
    pub async fn promote_staging(
        &self,
        from: &[String],
        to: &[String],
        remove_source: bool,
    ) -> Result<bool, AppError> {
        let (parent, name) = split_last(from)?;
        let mut sp = self.staging_parent(parent).await?;
        let Some(f) = sp.files.get(&name).cloned() else {
            return Err(AppError::NotFound);
        };
        // Only staged bytes can be promoted (a backup reference has none).
        let (Some(key), Some(hash)) = (f.staging_key.clone(), f.hash.clone()) else {
            return Err(AppError::NotFound);
        };
        trace!("vfs staging: promoting scratch bytes to picture");
        let created = self
            .finalize_write(
                to,
                ByteSource::Staging { key: key.clone() },
                &hash,
                f.size,
                f.content_type.as_deref(),
                // Preview-edit staging promotion (08_webdav_issues) carries no source mtime.
                None,
            )
            .await?;
        if remove_source {
            sp.files.remove(&name);
            self.save_staging_parent(parent, &sp).await?;
            let _ = self
                .state
                .storage
                .delete_object(&self.state.settings.get(keys::S3_BUCKET_STAGING), &key)
                .await;
        }
        Ok(created)
    }

    /// Relocate a scratch marker within the staging namespace (a MOVE of one scratch path to
    /// another, e.g. a client renaming its own temp).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, from = %from.join("/"), to = %to.join("/")))]
    pub async fn move_staging(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        let (fp, fname) = split_last(from)?;
        let (tp, tname) = split_last(to)?;
        let mut src = self.staging_parent(fp).await?;
        if let Some(mut f) = src.files.remove(&fname) {
            self.save_staging_parent(fp, &src).await?;
            f.name = tname.clone();
            let mut dst = self.staging_parent(tp).await?;
            dst.files.insert(tname, f);
            return self.save_staging_parent(tp, &dst).await;
        }
        if let Some(pos) = src.dirs.iter().position(|d| d == &fname) {
            src.dirs.remove(pos);
            self.save_staging_parent(fp, &src).await?;
            let mut dst = self.staging_parent(tp).await?;
            if !dst.dirs.iter().any(|d| d == &tname) {
                dst.dirs.push(tname);
            }
            return self.save_staging_parent(tp, &dst).await;
        }
        Ok(())
    }
}
