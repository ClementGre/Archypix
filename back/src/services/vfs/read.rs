
use crate::infra::s3;
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::services::hierarchy::{self, ResolvedDir};
use crate::services::pictures::{self, PictureVariant};
use crate::state::AppState;
use archypix_common::error::AppError;
use tracing::trace;
use uuid::Uuid;
use super::*;

impl<'a> Vfs<'a> {
    /// Load + resolve the hierarchy for a WebDAV session.
    #[tracing::instrument(skip(state), fields(user_id = %user_id, hierarchy_id = %hierarchy_id))]
    pub async fn load(
        state: &'a AppState,
        user_id: Uuid,
        hierarchy_id: Uuid,
        use_redirect: bool,
    ) -> Result<Vfs<'a>, AppError> {
        let (_row, _config, root) =
            hierarchy::load_resolved(&state.db, user_id, hierarchy_id).await?;
        Ok(Self {
            state,
            user_id,
            hierarchy_id,
            use_redirect,
            root,
        })
    }

    pub(super) fn dir(&self, segments: &[String]) -> Option<&ResolvedDir> {
        hierarchy::find_dir(&self.root, segments)
    }

    /// RFC 4331 capacity numbers for a collection PROPFIND (feature 22 §8.2): the owner's billed
    /// total (`quota-used-bytes`) and remaining space (`quota-available-bytes`; `None` = unlimited).
    pub async fn quota_props(&self) -> Result<(i64, Option<i64>), AppError> {
        let info = crate::services::storage::storage_info(
            &self.state.db,
            &self.state.settings,
            self.user_id,
        )
        .await?;
        Ok((info.used_bytes, info.available_bytes))
    }

    /// List the pictures that are direct files of `dir`, projected to entries.
    pub(super) async fn dir_files(&self, dir: &ResolvedDir) -> Result<Vec<VfsEntry>, AppError> {
        let Some(direct) = dir.direct.as_ref() else {
            return Ok(vec![]);
        };
        let filter = hierarchy::list_filter_for(direct, DIR_LIST_CAP);
        let (pics, _total) = PictureRepository::list(&self.state.db, self.user_id, &filter).await?;
        Ok(project_files(&pics, dir.naming, dir.writable))
    }

    /// List a directory: child directories first, then direct files. OS-junk sidecar files are
    /// merged in so they survive a round-trip (06_webdav.md §11). A `MKCOL`'d empty directory is an
    /// ordinary `show_when_empty` tag and is already in the resolved tree (34_tag_metadata.md §8.1).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn list_dir(&self, segments: &[String]) -> Result<Vec<VfsEntry>, AppError> {
        let mut out: Vec<VfsEntry> = Vec::new();
        let mut real_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        if let Some(dir) = self.dir(segments) {
            for c in &dir.children {
                real_names.insert(c.name.clone());
                out.push(dir_entry(&c.name, c.writable));
            }
            let files = self.dir_files(dir).await?;
            for f in &files {
                real_names.insert(f.name.clone());
            }
            out.extend(files);
        } else if !self.is_staging_dir(segments).await? {
            // Not a real directory, nor a known atomic-staging one.
            return Err(AppError::NotFound);
        }

        // Sidecar (OS-junk) files echoed back in the listing.
        for sc in self.sidecars(segments).await? {
            if real_names.insert(sc.name.clone()) {
                out.push(sidecar_entry(&sc));
            }
        }
        // Atomic-save scratch dirs/files echoed back until a rename promotes them (§1).
        let staging = self.staging_parent(segments).await?;
        for name in staging.dirs {
            if real_names.insert(name.clone()) {
                out.push(dir_entry(&name, true));
            }
        }
        for f in staging.files.into_values() {
            if real_names.insert(f.name.clone()) {
                out.push(staged_entry(&f));
            }
        }
        Ok(out)
    }

    /// Stat a path — a directory or a file (real or sidecar).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn stat(&self, segments: &[String]) -> Result<VfsEntry, AppError> {
        if segments.is_empty() {
            return Ok(dir_entry("", false));
        }
        if let Some(dir) = self.dir(segments) {
            return Ok(dir_entry(&dir.name, dir.writable));
        }
        match self.file_entry(segments).await {
            Ok(f) => return Ok(f),
            Err(AppError::NotFound) => {}
            Err(e) => return Err(e),
        }
        if self.is_staging_dir(segments).await? {
            let name = segments.last().cloned().unwrap_or_default();
            return Ok(dir_entry(&name, true));
        }
        if let Some(sc) = self.sidecar(segments).await? {
            return Ok(sidecar_entry(&sc));
        }
        if let Some(f) = self.staged_file(segments).await? {
            return Ok(staged_entry(&f));
        }
        Err(AppError::NotFound)
    }

    pub(super) async fn file_entry(&self, segments: &[String]) -> Result<VfsEntry, AppError> {
        let (parent, name) = split_last(segments)?;
        let dir = self.dir(parent).ok_or(AppError::NotFound)?;
        self.dir_files(dir)
            .await?
            .into_iter()
            .find(|f| f.name == name)
            .ok_or(AppError::NotFound)
    }

    /// Resolve a file read to a redirect or proxied bytes.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), picture_id))]
    pub async fn read_file(&self, segments: &[String]) -> Result<ReadTarget, AppError> {
        let entry = self.file_entry(segments).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        tracing::Span::current().record("picture_id", tracing::field::display(pid));
        self.read_picture(pid).await
    }

    /// Resolve a picture id to a redirect or proxied bytes (06_webdav.md §6). Shared by path reads
    /// and atomic-save backup-reference reads (08_webdav_issues.md §1.5).
    pub(super) async fn read_picture(&self, pid: Uuid) -> Result<ReadTarget, AppError> {
        let pic = PictureRepository::find_by_id(&self.state.db, pid)
            .await?
            .ok_or(AppError::NotFound)?;
        // Cross-instance received pictures always redirect (the bytes live on the owner's S3).
        if self.use_redirect || pic.remote_picture_id.is_some() {
            trace!("vfs read: redirect to presigned url");
            // `Original` always has a URL (no thumbnail-skipping); `None` would only mean missing.
            let url = pictures::presign_picture_variant(
                &self.state.db,
                self.state.cache.as_ref(),
                self.state.storage.as_ref(),
                &self.state.settings,
                &self.state.federation,
                self.user_id,
                pid,
                PictureVariant::Original,
            )
            .await?
            .ok_or(AppError::NotFound)?;
            Ok(ReadTarget::Redirect(url))
        } else {
            trace!("vfs read: proxy bytes from S3");
            let key = s3::picture_key(pic.local_user_id, pic.id);
            let data = self
                .state
                .storage
                .get_object(&self.state.settings.get(keys::S3_BUCKET_PICTURES), &key)
                .await?;
            Ok(ReadTarget::Bytes {
                data,
                mime: pic.mime_type,
            })
        }
    }

}
