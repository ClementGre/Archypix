//! The protocol-agnostic virtual filesystem over a hierarchy (06_webdav.md §5).
//!
//! [`Vfs`] resolves a hierarchy into a directory tree once (via
//! [`services::hierarchy::load_resolved`]) and exposes filesystem operations — list/stat/read
//! and the write taxonomy (PUT/DELETE/MOVE/COPY/MKCOL) — as tag mutations and uploads. The
//! WebDAV adapter in `api::webdav` is a thin shell over this; an SFTP adapter could reuse it.

use crate::domain::hierarchy::{NamingStrategy, SafeDeleteMode, TagOp, TagOpKind, WriteBack};
use crate::infra::error::AppError;
use crate::infra::s3;
use crate::repository::picture::PictureRepository;
use crate::repository::tag::TagRepository;
use crate::services::hierarchy::{self, ResolvedDir};
use crate::services::pictures::{self, PictureVariant};
use crate::state::AppState;
use chrono::{NaiveDateTime, Utc};
use sha2::{Digest, Sha256};
use tracing::trace;
use uuid::Uuid;

/// Cap on direct files listed per directory. WebDAV directories map to single tag paths, which
/// are not expected to hold tens of thousands of pictures; this is a guard, not pagination.
const DIR_LIST_CAP: i64 = 10_000;

/// A filesystem entry — a directory or a file projected from a picture.
pub struct VfsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: NaiveDateTime,
    /// `file_hash` for files (the WebDAV ETag); `None` for directories.
    pub etag: Option<String>,
    pub mime_type: Option<String>,
    pub picture_id: Option<Uuid>,
    pub writable: bool,
}

/// What a read resolves to: a redirect to a presigned URL, or proxied bytes (06_webdav.md §6).
pub enum ReadTarget {
    Redirect(String),
    Bytes { data: Vec<u8>, mime: Option<String> },
}

pub struct Vfs<'a> {
    state: &'a AppState,
    user_id: Uuid,
    use_redirect: bool,
    root: ResolvedDir,
}

impl<'a> Vfs<'a> {
    /// Load + resolve the hierarchy for a WebDAV session.
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
            use_redirect,
            root,
        })
    }

    fn dir(&self, segments: &[String]) -> Option<&ResolvedDir> {
        hierarchy::find_dir(&self.root, segments)
    }

    /// List the pictures that are direct files of `dir`, projected to entries.
    async fn dir_files(&self, dir: &ResolvedDir) -> Result<Vec<VfsEntry>, AppError> {
        let Some(direct) = dir.direct.as_ref() else {
            return Ok(vec![]);
        };
        let filter = hierarchy::list_filter_for(direct, DIR_LIST_CAP);
        let (pics, _total) = PictureRepository::list(&self.state.db, self.user_id, &filter).await?;
        Ok(project_files(&pics, dir.naming, dir.writable))
    }

    /// List a directory: child directories first, then direct files.
    pub async fn list_dir(&self, segments: &[String]) -> Result<Vec<VfsEntry>, AppError> {
        let dir = self.dir(segments).ok_or(AppError::NotFound)?;
        let mut out: Vec<VfsEntry> = dir
            .children
            .iter()
            .map(|c| dir_entry(&c.name, c.writable))
            .collect();
        out.extend(self.dir_files(dir).await?);
        Ok(out)
    }

    /// Stat a path — a directory or a file.
    pub async fn stat(&self, segments: &[String]) -> Result<VfsEntry, AppError> {
        if segments.is_empty() {
            return Ok(dir_entry("", false));
        }
        if let Some(dir) = self.dir(segments) {
            return Ok(dir_entry(&dir.name, dir.writable));
        }
        self.file_entry(segments).await
    }

    async fn file_entry(&self, segments: &[String]) -> Result<VfsEntry, AppError> {
        let (parent, name) = split_last(segments)?;
        let dir = self.dir(parent).ok_or(AppError::NotFound)?;
        self.dir_files(dir)
            .await?
            .into_iter()
            .find(|f| f.name == name)
            .ok_or(AppError::NotFound)
    }

    /// Resolve a file read to a redirect or proxied bytes.
    pub async fn read_file(&self, segments: &[String]) -> Result<ReadTarget, AppError> {
        let entry = self.file_entry(segments).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        let pic = PictureRepository::find_by_id(&self.state.db, pid)
            .await?
            .ok_or(AppError::NotFound)?;
        // Cross-instance received pictures always redirect (the bytes live on the owner's S3).
        if self.use_redirect || pic.remote_picture_id.is_some() {
            trace!(user_id = %self.user_id, picture_id = %pid, "vfs read: redirect to presigned url");
            let url = pictures::presign_picture_variant(
                &self.state.db,
                self.state.cache.as_ref(),
                self.state.storage.as_ref(),
                &self.state.config,
                &self.state.federation,
                self.user_id,
                pid,
                PictureVariant::Original,
            )
            .await?;
            Ok(ReadTarget::Redirect(url))
        } else {
            trace!(user_id = %self.user_id, picture_id = %pid, "vfs read: proxy bytes from S3");
            let key = s3::picture_key(pic.local_user_id, pic.id);
            let data = self
                .state
                .storage
                .get_object(&self.state.config.s3_bucket_pictures, &key)
                .await?;
            Ok(ReadTarget::Bytes {
                data,
                mime: pic.mime_type,
            })
        }
    }

    /// PUT a file. Returns `true` if a new resource was created, `false` if an existing one was
    /// overwritten/retagged. See §7–8.
    pub async fn put_file(
        &self,
        segments: &[String],
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Result<bool, AppError> {
        let (parent, name) = split_last(segments)?;
        let dir = self.dir(parent).ok_or(AppError::NotFound)?;

        // Overwrite: the name already maps to a picture in this directory.
        if let Some(existing) = self
            .dir_files(dir)
            .await?
            .into_iter()
            .find(|f| f.name == name)
        {
            let pid = existing.picture_id.ok_or(AppError::NotFound)?;
            let pic = PictureRepository::find_by_id(&self.state.db, pid)
                .await?
                .ok_or(AppError::NotFound)?;
            if pic.remote_picture_id.is_some() {
                return Err(AppError::Forbidden(
                    "cannot overwrite a received (shared) picture".into(),
                ));
            }
            trace!(user_id = %self.user_id, picture_id = %pid, name = %name, bytes = body.len(), "vfs put: overwrite existing picture");
            let key = s3::picture_key(pic.local_user_id, pic.id);
            // NB: versioning-on-overwrite is deferred (06_webdav.md §7.3) — in-place overwrite
            // then re-extract via gen_thumbnail.
            self.state
                .storage
                .put_object(
                    &self.state.config.s3_bucket_pictures,
                    &key,
                    body,
                    content_type,
                )
                .await?;
            crate::services::jobs::enqueue_thumbnail_job(&self.state.db, self.user_id, pid, false)
                .await?;
            self.state.pipeline_waker.wake(self.user_id);
            return Ok(false);
        }

        // New file — requires a writable directory.
        let wb = dir
            .write_back
            .as_ref()
            .ok_or_else(|| AppError::Forbidden("directory is read-only".into()))?
            .clone();
        // Use hash_file instead to hash a file in streaming
        let hash = archypix_common::hash::hash_bytes(&body)
            .ok_or_else(|| AppError::InternalServerError("failed to hash file".into()))?;

        // Dedupe: a relocate/copy a dumb client expressed as a fresh upload (§8).
        if let Some(p) =
            PictureRepository::find_owned_by_hash(&self.state.db, self.user_id, &hash, false)
                .await?
        {
            trace!(user_id = %self.user_id, picture_id = %p.id, %hash, "vfs put: hash matched live picture — retag instead of new upload");
            self.apply_add_ops(&wb.on_add, p.id).await?;
            self.state.pipeline_waker.wake(self.user_id);
            return Ok(false);
        }
        // Un-delete a recently trashed match (naive rename under fullDelete, §8).
        if let Some(p) =
            PictureRepository::find_owned_by_hash(&self.state.db, self.user_id, &hash, true).await?
        {
            trace!(user_id = %self.user_id, picture_id = %p.id, %hash, "vfs put: hash matched trashed picture — un-delete and retag");
            PictureRepository::set_deleted(&self.state.db, self.user_id, p.id, false).await?;
            self.apply_add_ops(&wb.on_add, p.id).await?;
            self.state.pipeline_waker.wake(self.user_id);
            return Ok(true);
        }

        // Genuine new picture: upload bytes, create the row + thumbnail job, then apply tags.
        let new_id = Uuid::new_v4();
        trace!(user_id = %self.user_id, picture_id = %new_id, name = %name, bytes = body.len(), "vfs put: ingest new picture");
        let key = s3::picture_key(self.user_id, new_id);
        let size = body.len() as i64;
        self.state
            .storage
            .put_object(
                &self.state.config.s3_bucket_pictures,
                &key,
                body,
                content_type,
            )
            .await?;
        let mut tx = self
            .state
            .db
            .begin()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        PictureRepository::create(
            &mut *tx,
            new_id,
            self.user_id,
            Some(&name),
            content_type,
            Some(size),
            None,
            None,
            None,
            None,
        )
        .await?;
        crate::services::jobs::enqueue_thumbnail_job(&mut *tx, self.user_id, new_id, true).await?;
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        self.apply_add_ops(&wb.on_add, new_id).await?;
        self.state.pipeline_waker.wake(self.user_id);
        Ok(true)
    }

    /// DELETE a file per the directory's `safeDeleteMode` (§7.1).
    pub async fn delete(&self, segments: &[String]) -> Result<(), AppError> {
        let entry = self.file_entry(segments).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        let (parent, _) = split_last(segments)?;
        let dir = self.dir(parent).ok_or(AppError::NotFound)?;
        match dir.safe_delete_mode {
            SafeDeleteMode::FullDelete => {
                trace!(user_id = %self.user_id, picture_id = %pid, "vfs delete: fullDelete (trash picture)");
                // Trash (received pictures too — local deleted_at only).
                PictureRepository::set_deleted(&self.state.db, self.user_id, pid, true).await?;
            }
            SafeDeleteMode::SingleBranch => {
                trace!(user_id = %self.user_id, picture_id = %pid, "vfs delete: singleBranch (apply onRemove)");
                let wb = dir.write_back.as_ref().ok_or_else(|| {
                    AppError::Forbidden(
                        "read-only directory; singleBranch delete not allowed".into(),
                    )
                })?;
                self.apply_remove_ops(&wb.on_remove, pid).await?;
            }
        }
        self.state.pipeline_waker.wake(self.user_id);
        Ok(())
    }

    /// MOVE: rename within a directory, or re-file across directories (§7.1).
    pub async fn move_(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        let entry = self.file_entry(from).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        let (from_parent, _) = split_last(from)?;
        let (to_parent, to_name) = split_last(to)?;

        if from_parent == to_parent {
            // Rename — set the filename (meaningful for naming=original).
            trace!(user_id = %self.user_id, picture_id = %pid, new_name = %to_name, "vfs move: rename within directory");
            PictureRepository::set_filename(&self.state.db, self.user_id, pid, &to_name).await?;
            self.state.pipeline_waker.wake(self.user_id);
            return Ok(());
        }
        trace!(user_id = %self.user_id, picture_id = %pid, from = %from_parent.join("/"), to = %to_parent.join("/"), "vfs move: re-file across directories");

        // Re-file: remove from source, add to destination.
        let src = self.dir(from_parent).ok_or(AppError::NotFound)?;
        let src_wb = src
            .write_back
            .as_ref()
            .ok_or_else(|| AppError::Forbidden("source directory is read-only".into()))?
            .clone();
        let dst = self.dir(to_parent).ok_or(AppError::NotFound)?;
        let dst_wb = dst
            .write_back
            .as_ref()
            .ok_or_else(|| AppError::Forbidden("destination directory is read-only".into()))?
            .clone();
        self.apply_remove_ops(&src_wb.on_remove, pid).await?;
        self.apply_add_ops(&dst_wb.on_add, pid).await?;
        self.state.pipeline_waker.wake(self.user_id);
        Ok(())
    }

    /// COPY: the picture gains the destination directory's tags (becomes multi-tagged).
    pub async fn copy(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        let entry = self.file_entry(from).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        let (to_parent, _) = split_last(to)?;
        let dst = self.dir(to_parent).ok_or(AppError::NotFound)?;
        let dst_wb = dst
            .write_back
            .as_ref()
            .ok_or_else(|| AppError::Forbidden("destination directory is read-only".into()))?
            .clone();
        trace!(user_id = %self.user_id, picture_id = %pid, to = %to_parent.join("/"), "vfs copy: add destination tags");
        self.apply_add_ops(&dst_wb.on_add, pid).await?;
        self.state.pipeline_waker.wake(self.user_id);
        Ok(())
    }

    /// MKCOL: structure is tag-derived, so directory creation is only accepted (transiently)
    /// under a writable parent; persisting an empty directory has nothing to store (§9). The
    /// brand-new-subdir auto-tag-on-write case is deferred.
    pub fn mkcol(&self, segments: &[String]) -> Result<(), AppError> {
        let (parent, _) = split_last(segments)?;
        let dir = self.dir(parent).ok_or(AppError::NotFound)?;
        if dir.writable {
            trace!(user_id = %self.user_id, path = %segments.join("/"), "vfs mkcol: accepted (transient — not persisted)");
            Ok(())
        } else {
            Err(AppError::Forbidden(
                "cannot create directories in a read-only part of the hierarchy".into(),
            ))
        }
    }

    async fn apply_add_ops(&self, ops: &[TagOp], pid: Uuid) -> Result<(), AppError> {
        let (assigns, removes) = split_ops(ops);
        trace!(user_id = %self.user_id, picture_id = %pid, ?assigns, ?removes, "vfs: apply onAdd ops");
        if !assigns.is_empty() {
            TagRepository::batch_assign(&self.state.db, self.user_id, &[pid], &assigns).await?;
        }
        if !removes.is_empty() {
            TagRepository::batch_remove(&self.state.db, self.user_id, &[pid], &removes).await?;
        }
        Ok(())
    }

    /// Apply `onRemove` ops, rejecting with 409 if a removed tag would survive because a live
    /// service still asserts it (§7.2).
    async fn apply_remove_ops(&self, ops: &[TagOp], pid: Uuid) -> Result<(), AppError> {
        let (assigns, removes) = split_ops(ops);
        trace!(user_id = %self.user_id, picture_id = %pid, ?removes, ?assigns, "vfs: apply onRemove ops");
        if TagRepository::has_non_manual_tag_under(&self.state.db, pid, &removes).await? {
            trace!(user_id = %self.user_id, picture_id = %pid, "vfs: onRemove rejected — non-manual tag still asserted (409)");
            return Err(AppError::Conflict(
                "a tagging service still asserts this tag — cannot remove via WebDAV".into(),
            ));
        }
        if !removes.is_empty() {
            TagRepository::batch_remove(&self.state.db, self.user_id, &[pid], &removes).await?;
        }
        if !assigns.is_empty() {
            TagRepository::batch_assign(&self.state.db, self.user_id, &[pid], &assigns).await?;
        }
        Ok(())
    }
}

fn split_ops(ops: &[TagOp]) -> (Vec<String>, Vec<String>) {
    let assigns = ops
        .iter()
        .filter(|o| o.op == TagOpKind::Assign)
        .map(|o| o.path.clone())
        .collect();
    let removes = ops
        .iter()
        .filter(|o| o.op == TagOpKind::Remove)
        .map(|o| o.path.clone())
        .collect();
    (assigns, removes)
}

fn dir_entry(name: &str, writable: bool) -> VfsEntry {
    VfsEntry {
        name: name.to_string(),
        is_dir: true,
        size: 0,
        modified: Utc::now().naive_utc(),
        etag: None,
        mime_type: None,
        picture_id: None,
        writable,
    }
}

/// Split path segments into (parent, last). Errors on empty (no last segment).
fn split_last(segments: &[String]) -> Result<(&[String], String), AppError> {
    match segments.split_last() {
        Some((last, parent)) => Ok((parent, last.clone())),
        None => Err(AppError::NotFound),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Project a directory's pictures to file entries, applying the naming strategy and
/// disambiguating in-directory name collisions with the picture-id suffix (§8 naming).
pub fn project_files(
    pics: &[crate::domain::picture::Picture],
    naming: NamingStrategy,
    writable: bool,
) -> Vec<VfsEntry> {
    use std::collections::HashMap;
    // Stable order (by id) so disambiguation is deterministic and reversible.
    let mut order: Vec<&crate::domain::picture::Picture> = pics.iter().collect();
    order.sort_by_key(|p| p.id);

    let mut counts: HashMap<String, usize> = HashMap::new();
    let bases: Vec<String> = order.iter().map(|p| base_name(p, naming)).collect();
    for b in &bases {
        *counts.entry(b.to_lowercase()).or_default() += 1;
    }

    order
        .iter()
        .zip(bases.iter())
        .map(|(p, base)| {
            let name = if counts.get(&base.to_lowercase()).copied().unwrap_or(0) > 1 {
                disambiguate(base, p.id)
            } else {
                base.clone()
            };
            VfsEntry {
                name,
                is_dir: false,
                size: p.file_size.unwrap_or(0).max(0) as u64,
                modified: p.updated_at,
                etag: p.file_hash.clone(),
                mime_type: p.mime_type.clone(),
                picture_id: Some(p.id),
                writable,
            }
        })
        .collect()
}

fn base_name(p: &crate::domain::picture::Picture, naming: NamingStrategy) -> String {
    let ext = extension(p);
    match naming {
        NamingStrategy::Original => p
            .filename
            .clone()
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| format!("{}.{}", p.id, ext)),
        NamingStrategy::Date => match p.captured_at {
            Some(c) => format!("{}.{}", c.format("%Y-%m-%d_%H%M%S"), ext),
            None => format!("{}.{}", p.id, ext),
        },
        NamingStrategy::Id => format!("{}.{}", p.id, ext),
    }
}

fn disambiguate(base: &str, id: Uuid) -> String {
    let suffix = &id.simple().to_string()[..6];
    match base.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}-{suffix}.{ext}"),
        None => format!("{base}-{suffix}"),
    }
}

fn extension(p: &crate::domain::picture::Picture) -> String {
    if let Some(f) = &p.filename {
        if let Some((_, ext)) = f.rsplit_once('.') {
            if !ext.is_empty() && ext.len() <= 5 {
                return ext.to_lowercase();
            }
        }
    }
    match p.mime_type.as_deref() {
        Some("image/jpeg") => "jpg",
        Some("image/png") => "png",
        Some("image/webp") => "webp",
        Some("image/gif") => "gif",
        Some("image/tiff") => "tiff",
        Some("image/heic") => "heic",
        _ => "bin",
    }
    .to_string()
}
