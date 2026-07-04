//! The protocol-agnostic virtual filesystem over a hierarchy (06_webdav.md §5).
//!
//! [`Vfs`] resolves a hierarchy into a directory tree once (via
//! [`services::hierarchy::load_resolved`]) and exposes filesystem operations — list/stat/read
//! and the write taxonomy (PUT/DELETE/MOVE/COPY/MKCOL) — as tag mutations and uploads. The
//! WebDAV adapter in `api::webdav` is a thin shell over this; an SFTP adapter could reuse it.

use crate::domain::hierarchy::{NamingStrategy, SafeDeleteMode, TagOp, TagOpKind};
use crate::domain::tag::TagPath;
use crate::infra::error::AppError;
use crate::infra::redis::{RedisKey, cache_get_json, cache_set_json_ex};
use crate::infra::s3;
use crate::repository::picture::PictureRepository;
use crate::repository::tag::TagRepository;
use crate::repository::user_settings::UserSettingsRepository;
use crate::services::hierarchy::{self, ResolvedDir};
use crate::services::pictures::{self, PictureVariant};
use crate::state::AppState;
use base64::Engine as _;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::trace;
use uuid::Uuid;

/// Cap on direct files listed per directory. WebDAV directories map to single tag paths, which
/// are not expected to hold tens of thousands of pictures; this is a guard, not pagination.
const DIR_LIST_CAP: i64 = 10_000;

/// TTL for the transient Redis markers (pending `MKCOL` dirs §9, OS-junk sidecars §11). Long
/// enough to survive a sync session; a real file landing converts a pending dir to a tag, and
/// the markers GC by TTL otherwise.
const TRANSIENT_TTL_SECS: u64 = 86_400;

/// Upper bound on an echoed-back OS-junk sidecar (`.DS_Store`, …). Larger bodies are accepted but
/// not stored — they are never pictures and not worth keeping in Redis.
const SIDECAR_MAX_BYTES: usize = 1024 * 1024;

/// A brand-new path resolved against the live tree: either an existing directory, or an extension
/// of a `mirror` node by new trailing segments that map to a deeper tag (06_webdav.md §9).
enum PathResolution<'d> {
    Existing(&'d ResolvedDir),
    /// The deepest tag to assign (`tagRoot + new segments`), already validated.
    MirrorExtension {
        tag: String,
    },
}

/// Where a finalized picture's bytes come from: a local temp file (a direct PUT) or an object
/// already in the staging bucket (an atomic-save promotion, 08_webdav_issues.md §1.6).
enum ByteSource<'p> {
    LocalTemp(&'p Path),
    Staging { key: String },
}

impl ByteSource<'_> {
    /// Move the bytes into their final `dst_bucket/dst_key`: upload the temp file, or a server-side
    /// S3 copy from the staging bucket (no re-stream — the hash is already known).
    async fn copy_to(
        &self,
        state: &AppState,
        dst_bucket: &str,
        dst_key: &str,
        content_type: Option<&str>,
    ) -> Result<(), AppError> {
        match self {
            ByteSource::LocalTemp(path) => {
                state
                    .storage
                    .put_object_file(dst_bucket, dst_key, path, content_type)
                    .await
            }
            ByteSource::Staging { key } => {
                state
                    .storage
                    .copy_object(&state.config.s3_bucket_staging, key, dst_bucket, dst_key)
                    .await
            }
        }
    }
}

/// A scratch namespace under one parent directory (08_webdav_issues.md §1): temp sub-directories
/// created by `MKCOL` and staged files written by `PUT`, echoed back in listings until a terminal
/// rename promotes them or the TTL sweeps them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StagingParent {
    dirs: Vec<String>,
    files: HashMap<String, StagedFile>,
}

/// A single atomic-save scratch file. Either carries staged bytes (`staging_key`) or is a backup
/// reference to an existing picture (`picture_ref`, the "move original out of the way" step).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StagedFile {
    name: String,
    size: i64,
    /// Unix seconds.
    mtime: i64,
    /// SHA-256 of the staged bytes (the ETag); `None` for a backup reference.
    hash: Option<String>,
    content_type: Option<String>,
    /// Key of the staged object in the staging bucket; `None` for a backup reference.
    staging_key: Option<String>,
    /// Set when this entry references an existing picture instead of staged bytes.
    picture_ref: Option<Uuid>,
}

/// A stored OS-junk sidecar file (06_webdav.md §11). Echoed back in listings; never a picture.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Sidecar {
    name: String,
    size: u64,
    /// Unix seconds.
    mtime: i64,
    content_type: Option<String>,
    /// Base64-encoded bytes (kept small; see [`SIDECAR_MAX_BYTES`]).
    data_b64: String,
}

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
    hierarchy_id: Uuid,
    use_redirect: bool,
    root: ResolvedDir,
}

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

    /// List a directory: child directories first, then direct files. Brand-new mirror
    /// sub-directories created via `MKCOL` (Redis pending markers) and OS-junk sidecar files are
    /// merged in so they survive a round-trip until a real file lands (06_webdav.md §9, §11).
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
        } else if !self.is_pending_dir(segments).await? && !self.is_staging_dir(segments).await? {
            // Not a real directory, nor a known pending/atomic-staging one.
            return Err(AppError::NotFound);
        }

        // Pending sub-directories (writable, since they only exist under a mirror).
        for name in self.pending_dir_children(segments).await? {
            if real_names.insert(name.clone()) {
                out.push(dir_entry(&name, true));
            }
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

    /// Stat a path — a directory (real or pending) or a file (real or sidecar).
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
        if self.is_pending_dir(segments).await? || self.is_staging_dir(segments).await? {
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
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), picture_id))]
    pub async fn read_file(&self, segments: &[String]) -> Result<ReadTarget, AppError> {
        let entry = self.file_entry(segments).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        tracing::Span::current().record("picture_id", tracing::field::display(pid));
        self.read_picture(pid).await
    }

    /// Resolve a picture id to a redirect or proxied bytes (06_webdav.md §6). Shared by path reads
    /// and atomic-save backup-reference reads (08_webdav_issues.md §1.5).
    async fn read_picture(&self, pid: Uuid) -> Result<ReadTarget, AppError> {
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
                &self.state.config,
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
                .get_object(&self.state.config.s3_bucket_pictures, &key)
                .await?;
            Ok(ReadTarget::Bytes {
                data,
                mime: pic.mime_type,
            })
        }
    }

    /// PUT a file. The request body has already been streamed to `temp_path` with its SHA-256
    /// computed inline (06_webdav.md §7); `hash`/`size` describe those streamed bytes. Returns
    /// `true` if a new resource was created, `false` if an existing one was overwritten/retagged.
    /// See §7–8.
    pub async fn put_file(
        &self,
        segments: &[String],
        temp_path: &Path,
        hash: &str,
        size: i64,
        content_type: Option<&str>,
    ) -> Result<bool, AppError> {
        self.finalize_write(
            segments,
            ByteSource::LocalTemp(temp_path),
            hash,
            size,
            content_type,
        )
        .await
    }

    /// Finalize a write to `segments` from either a local temp file or a staged object
    /// (08_webdav_issues.md §1.6): overwrite an existing picture (versioned), dedupe/relocate on a
    /// hash hit, or ingest a genuinely new picture and apply the target's tags.
    #[tracing::instrument(
        skip(self, source),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), hash = %hash, bytes = size, picture_id)
    )]
    async fn finalize_write(
        &self,
        segments: &[String],
        source: ByteSource<'_>,
        hash: &str,
        size: i64,
        content_type: Option<&str>,
    ) -> Result<bool, AppError> {
        let (parent, name) = split_last(segments)?;

        // A zero-byte PUT is never a valid picture — Finder/Explorer issue one to create a
        // placeholder before writing the real bytes in a second PUT. Accept it but ingest
        // nothing, so empty objects never reach S3 or the picture table.
        if size == 0 {
            trace!("vfs put: empty body — accepted without ingesting");
            return Ok(true);
        }

        // Resolve the destination directory: an existing one, or a brand-new mirror sub-path
        // whose new segments mint a deeper tag (06_webdav.md §9).
        let target = self.resolve_path(parent)?;

        // Overwrite is only possible inside an existing directory.
        if let PathResolution::Existing(dir) = &target {
            let dir = *dir;
            if let Some(existing) = self
                .dir_files(dir)
                .await?
                .into_iter()
                .find(|f| f.name == name)
            {
                let pid = existing.picture_id.ok_or(AppError::NotFound)?;
                tracing::Span::current().record("picture_id", tracing::field::display(pid));
                let pic = PictureRepository::find_by_id(&self.state.db, pid)
                    .await?
                    .ok_or(AppError::NotFound)?;
                if pic.remote_picture_id.is_some() {
                    return Err(AppError::Forbidden(
                        "cannot overwrite a received (shared) picture".into(),
                    ));
                }

                // Idempotent re-PUT: a dumb sync client re-uploading identical bytes.
                if pic.file_hash.as_deref() == Some(hash) {
                    trace!("vfs put: identical bytes (hash match) — no-op overwrite");
                    return Ok(false);
                }

                trace!("vfs put: overwrite existing picture");

                // Versioning on overwrite (§7.3): snapshot the current bytes per the user's
                // versioning_mode before replacing them.
                let settings =
                    UserSettingsRepository::get_or_default(&self.state.db, self.user_id).await?;
                pictures::snapshot_version_on_overwrite(
                    &self.state.db,
                    self.state.storage.as_ref(),
                    &self.state.config,
                    settings.versioning_mode,
                    &pic,
                )
                .await?;

                let key = s3::picture_key(pic.local_user_id, pic.id);
                source
                    .copy_to(
                        self.state,
                        &self.state.config.s3_bucket_pictures,
                        &key,
                        content_type,
                    )
                    .await?;
                // Set the new hash/size inline so the ETag is correct before gen_thumbnail re-extracts.
                PictureRepository::set_file_hash(&self.state.db, pid, hash, Some(size)).await?;
                // is_initial = true so the exif is re-extracted from the new bytes. Keyed on the
                // new file hash so the overwrite is not blocked by the first-upload extraction job.
                crate::services::jobs::enqueue_thumbnail_job(
                    &self.state.db,
                    self.user_id,
                    pid,
                    true,
                    Some(hash),
                )
                .await?;
                self.state.routines.pipeline.trigger_debounced(self.user_id);
                return Ok(false);
            }
        }

        // New file — the destination's onAdd ops (existing writable dir's op-list, or the
        // synthesized mirror auto-tag assign).
        let on_add = self.on_add_ops(&target)?;

        // Dedupe: a relocate/copy a dumb client expressed as a fresh upload (§8). If the picture
        // gains the directory's tag (it wasn't already here) it's a genuine new resource for this
        // path → 201 Created; if it already had the tag the PUT is a true no-op → 204 No Content.
        if let Some(p) =
            PictureRepository::find_owned_by_hash(&self.state.db, self.user_id, hash, false).await?
        {
            tracing::Span::current().record("picture_id", tracing::field::display(p.id));
            trace!("vfs put: hash matched live picture — retag instead of new upload");
            let added = self.apply_add_ops(&on_add, p.id).await?;
            self.clear_pending_dir(parent).await;
            self.state.routines.pipeline.trigger_debounced(self.user_id);
            return Ok(added);
        }
        // Un-delete a recently trashed match (naive rename under fullDelete, §8).
        if let Some(p) =
            PictureRepository::find_owned_by_hash(&self.state.db, self.user_id, hash, true).await?
        {
            tracing::Span::current().record("picture_id", tracing::field::display(p.id));
            trace!("vfs put: hash matched trashed picture — un-delete and retag");
            PictureRepository::set_deleted(&self.state.db, self.user_id, p.id, false).await?;
            self.apply_add_ops(&on_add, p.id).await?;
            self.clear_pending_dir(parent).await;
            self.state.routines.pipeline.trigger_debounced(self.user_id);
            return Ok(true);
        }

        // Genuine new picture: stream bytes to S3, create the row + thumbnail job, then apply tags.
        let new_id = Uuid::new_v4();
        tracing::Span::current().record("picture_id", tracing::field::display(new_id));
        trace!("vfs put: ingest new picture");
        let key = s3::picture_key(self.user_id, new_id);
        source
            .copy_to(
                self.state,
                &self.state.config.s3_bucket_pictures,
                &key,
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
        // Persist the inline hash so the ETag is correct and a quick re-upload dedupes (§8).
        PictureRepository::set_file_hash(&mut *tx, new_id, hash, Some(size)).await?;
        crate::services::jobs::enqueue_thumbnail_job(
            &mut *tx,
            self.user_id,
            new_id,
            true,
            Some(hash),
        )
        .await?;
        tx.commit()
            .await
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        self.apply_add_ops(&on_add, new_id).await?;
        // A real file now lives here, so the directory is a live tag — drop any pending marker.
        self.clear_pending_dir(parent).await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(true)
    }

    /// Resolve a (possibly not-yet-existing) directory path to either an existing [`ResolvedDir`]
    /// or a `mirror` extension that mints a deeper tag (06_webdav.md §9). The new trailing
    /// segments must be valid tag labels, and the nearest existing ancestor must be a writable
    /// `mirror` node — otherwise the write is rejected.
    fn resolve_path(&self, segments: &[String]) -> Result<PathResolution<'_>, AppError> {
        if let Some(dir) = self.dir(segments) {
            return Ok(PathResolution::Existing(dir));
        }
        // Walk up to the nearest existing ancestor.
        for i in (0..segments.len()).rev() {
            let Some(anc) = self.dir(&segments[..i]) else {
                continue;
            };
            // The base tag + its writability: a `mirror` directory extends its own tag; a
            // container hoisting a `keepDir=false` mirror (root/static/query) maps a brand-new
            // child directory to that mirror's tagRoot (feature 18 §11).
            let (base, writable) = if let Some(t) = anc.mirror_tag.as_ref() {
                (t.clone(), anc.writable)
            } else if let Some((t, w)) = anc.new_child_mirror.as_ref() {
                (t.clone(), *w)
            } else {
                return Err(AppError::Forbidden(
                    "cannot create directories outside a mirror node".into(),
                ));
            };
            if !writable {
                return Err(AppError::Forbidden(
                    "this part of the hierarchy is read-only".into(),
                ));
            }
            // Slugify each new segment into a valid tag label so a filesystem folder name with
            // spaces/punctuation (Finder's "dossier sans titre") still mints a tag (§9) instead of
            // being rejected — the client can't always rename before the first write.
            let labels = segments[i..]
                .iter()
                .map(|s| TagPath::slugify_label(s))
                .collect::<Vec<_>>()
                .join(".");
            let candidate = format!("{base}.{labels}");
            // Slugified labels are valid; `parse` still rejects a reserved (`SharedToMe`) prefix.
            let tag = TagPath::parse(&candidate, false)
                .map_err(AppError::Conflict)?
                .as_ltree()
                .to_string();
            return Ok(PathResolution::MirrorExtension { tag });
        }
        Err(AppError::NotFound)
    }

    /// The onAdd ops to apply when a picture lands in `target`: the existing directory's op-list,
    /// or a single assign of the synthesized mirror tag.
    fn on_add_ops(&self, target: &PathResolution<'_>) -> Result<Vec<TagOp>, AppError> {
        match target {
            PathResolution::Existing(dir) => Ok(dir
                .write_back
                .as_ref()
                .ok_or_else(|| AppError::Forbidden("directory is read-only".into()))?
                .on_add
                .clone()),
            PathResolution::MirrorExtension { tag } => Ok(vec![TagOp {
                op: TagOpKind::Assign,
                path: tag.clone(),
            }]),
        }
    }

    /// DELETE a file per the directory's `safeDeleteMode` (§7.1).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), picture_id))]
    pub async fn delete(&self, segments: &[String]) -> Result<(), AppError> {
        // Deleting an empty, still-pending MKCOL directory just drops its Redis marker.
        if self.dir(segments).is_none() && self.is_pending_dir(segments).await? {
            trace!("vfs delete: drop pending directory marker");
            self.clear_pending_dir(segments).await;
            return Ok(());
        }
        // Deleting a real directory: accept it if the directory is empty.
        if self.dir(segments).is_some() {
            if self.list_dir(segments).await?.is_empty() {
                trace!("vfs delete: empty directory — no-op success");
                self.clear_pending_dir(segments).await;
                return Ok(());
            }
            return Err(AppError::Conflict(
                "cannot delete a non-empty directory".into(),
            ));
        }
        let entry = self.file_entry(segments).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        tracing::Span::current().record("picture_id", tracing::field::display(pid));
        let (parent, _) = split_last(segments)?;
        let dir = self.dir(parent).ok_or(AppError::NotFound)?;
        // safeDeleteMode is only meaningful when the directory is effectively writable (feature
        // 18 §5.3): a read-only directory has no tag to single-branch-remove, so delete is always
        // a fullDelete (trash).
        let mode = if dir.writable {
            dir.safe_delete_mode
        } else {
            SafeDeleteMode::FullDelete
        };
        match mode {
            SafeDeleteMode::FullDelete => {
                trace!("vfs delete: fullDelete (trash picture)");
                // Trash (received pictures too — local deleted_at only).
                PictureRepository::set_deleted(&self.state.db, self.user_id, pid, true).await?;
            }
            SafeDeleteMode::SingleBranch => {
                trace!("vfs delete: singleBranch (apply onRemove)");
                let wb = dir.write_back.as_ref().ok_or_else(|| {
                    AppError::Forbidden(
                        "read-only directory; singleBranch delete not allowed".into(),
                    )
                })?;
                self.apply_remove_ops(&wb.on_remove, pid).await?;
            }
        }
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(())
    }

    /// MOVE: rename within a directory, or re-file across directories (§7.1).
    #[tracing::instrument(
        skip(self),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, from = %from.join("/"), to = %to.join("/"), picture_id)
    )]
    pub async fn move_(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        // Renaming a freshly-created (empty, pending) directory just moves its Redis marker —
        // Finder creates "dossier sans titre" then immediately MOVEs it to the chosen name.
        if self.dir(from).is_none() && self.is_pending_dir(from).await? {
            trace!("vfs move: rename pending directory");
            self.clear_pending_dir(from).await;
            self.add_pending_dir(to).await?;
            return Ok(());
        }
        let entry = self.file_entry(from).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        tracing::Span::current().record("picture_id", tracing::field::display(pid));
        let (from_parent, _) = split_last(from)?;
        let (to_parent, to_name) = split_last(to)?;

        if from_parent == to_parent {
            // Rename — set the filename (meaningful for naming=original).
            trace!("vfs move: rename within directory");
            PictureRepository::set_filename(&self.state.db, self.user_id, pid, &to_name).await?;
            self.state.routines.pipeline.trigger_debounced(self.user_id);
            return Ok(());
        }
        trace!("vfs move: re-file across directories");

        // Re-file: remove from source, add to destination (existing dir or mirror extension §9).
        let src = self.dir(from_parent).ok_or(AppError::NotFound)?;
        let src_wb = src
            .write_back
            .as_ref()
            .ok_or_else(|| AppError::Forbidden("source directory is read-only".into()))?
            .clone();
        let dst_on_add = self.on_add_ops(&self.resolve_path(to_parent)?)?;
        self.apply_remove_ops(&src_wb.on_remove, pid).await?;
        self.apply_add_ops(&dst_on_add, pid).await?;
        self.clear_pending_dir(to_parent).await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(())
    }

    /// COPY: the picture gains the destination directory's tags (becomes multi-tagged). The
    /// destination may be a brand-new mirror sub-path (§9).
    #[tracing::instrument(
        skip(self),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, from = %from.join("/"), to = %to.join("/"), picture_id)
    )]
    pub async fn copy(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        let entry = self.file_entry(from).await?;
        let pid = entry.picture_id.ok_or(AppError::NotFound)?;
        tracing::Span::current().record("picture_id", tracing::field::display(pid));
        let (to_parent, _) = split_last(to)?;
        let dst_on_add = self.on_add_ops(&self.resolve_path(to_parent)?)?;
        trace!("vfs copy: add destination tags");
        self.apply_add_ops(&dst_on_add, pid).await?;
        self.clear_pending_dir(to_parent).await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(())
    }

    /// MKCOL: directories are tag-derived. Under a writable `mirror` node a brand-new sub-path is
    /// recorded as a transient Redis pending marker so PROPFIND shows the empty directory until a
    /// file lands and mints the real tag (06_webdav.md §9). `static`/`query` structure is fixed,
    /// and an already-existing path is rejected.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn mkcol(&self, segments: &[String]) -> Result<(), AppError> {
        if self.dir(segments).is_some() || self.is_pending_dir(segments).await? {
            return Err(AppError::Conflict("directory already exists".into()));
        }
        // A drop inbox is a leaf — MKCOL inside it is not allowed (feature 18 §4).
        if let Some((_, parent)) = segments.split_last() {
            if self.dir(parent).is_some_and(|d| d.always_visible) {
                return Err(AppError::MethodNotAllowed(
                    "cannot create a directory inside a drop inbox".into(),
                ));
            }
        }
        match self.resolve_path(segments)? {
            PathResolution::Existing(_) => {
                Err(AppError::Conflict("directory already exists".into()))
            }
            PathResolution::MirrorExtension { .. } => {
                trace!("vfs mkcol: recorded pending mirror sub-directory");
                self.add_pending_dir(segments).await?;
                Ok(())
            }
        }
    }

    // `user_id`/`picture_id` are already on the calling span (put_file/move_/copy record
    // `picture_id` before reaching here) — no fields of our own to add.
    /// Returns whether the picture actually gained a tag (≥1 row inserted)
    #[tracing::instrument(skip_all)]
    async fn apply_add_ops(&self, ops: &[TagOp], pid: Uuid) -> Result<bool, AppError> {
        let (assigns, removes) = split_ops(ops);
        // Case-insensitive write-side reuse (§10c): fold each assigned tag onto an existing
        // case-variant sibling so a case-insensitive client never mints a case-only duplicate.
        let assigns = self.fold_case(assigns).await?;
        trace!(?assigns, ?removes, "vfs: apply onAdd ops");
        let mut inserted = 0u64;
        if !assigns.is_empty() {
            inserted =
                TagRepository::batch_assign(&self.state.db, self.user_id, &[pid], &assigns).await?;
        }
        if !removes.is_empty() {
            TagRepository::batch_remove(&self.state.db, self.user_id, &[pid], &removes).await?;
        }
        Ok(inserted > 0)
    }

    /// Fold assigned tag paths onto existing case-variant tags (06_webdav.md §10c). Loads the
    /// user's distinct tag paths once and rewrites each candidate's casing to reuse an existing
    /// sibling that differs only by case.
    async fn fold_case(&self, assigns: Vec<String>) -> Result<Vec<String>, AppError> {
        if assigns.is_empty() {
            return Ok(assigns);
        }
        let existing = TagRepository::list_paths_by_user(&self.state.db, self.user_id).await?;
        Ok(assigns
            .into_iter()
            .map(|p| crate::domain::hierarchy::reuse_existing_case(&p, &existing))
            .collect())
    }

    /// Apply `onRemove` ops, rejecting with 409 if a removed tag would survive because a live
    /// service still asserts it (§7.2).
    // `user_id`/`picture_id` are already on the calling span (delete/move_ record `picture_id`
    // before reaching here) — no fields of our own to add.
    #[tracing::instrument(skip_all)]
    async fn apply_remove_ops(&self, ops: &[TagOp], pid: Uuid) -> Result<(), AppError> {
        let (assigns, removes) = split_ops(ops);
        trace!(?removes, ?assigns, "vfs: apply onRemove ops");
        if TagRepository::has_non_manual_tag_under(&self.state.db, pid, &removes).await? {
            trace!("vfs: onRemove rejected — non-manual tag still asserted (409)");
            return Err(AppError::Conflict(
                "a tagging service still asserts this tag — cannot remove via WebDAV".into(),
            ));
        }
        if !removes.is_empty() {
            TagRepository::batch_remove(&self.state.db, self.user_id, &[pid], &removes).await?;
        }
        let assigns = self.fold_case(assigns).await?;
        if !assigns.is_empty() {
            TagRepository::batch_assign(&self.state.db, self.user_id, &[pid], &assigns).await?;
        }
        Ok(())
    }

    // ── Pending mirror sub-directories (MKCOL, §9) ────────────────────────────────

    /// Pending child directory names recorded under `parent` (06_webdav.md §9).
    async fn pending_dir_children(&self, parent: &[String]) -> Result<Vec<String>, AppError> {
        let key = path_key(parent);
        Ok(cache_get_json::<Vec<String>>(
            self.state.cache.as_ref(),
            RedisKey::WebdavPendingDir(self.hierarchy_id, &key),
        )
        .await?
        .unwrap_or_default())
    }

    /// Whether `segments` names a pending (MKCOL'd, not-yet-real) directory.
    async fn is_pending_dir(&self, segments: &[String]) -> Result<bool, AppError> {
        let Some((name, parent)) = segments.split_last() else {
            return Ok(false);
        };
        Ok(self
            .pending_dir_children(parent)
            .await?
            .iter()
            .any(|n| n == name))
    }

    /// Record `segments` as a pending child directory of its parent.
    async fn add_pending_dir(&self, segments: &[String]) -> Result<(), AppError> {
        let (parent, name) = split_last(segments)?;
        let key = path_key(parent);
        let mut set = self.pending_dir_children(parent).await?;
        if !set.iter().any(|n| n == &name) {
            set.push(name);
        }
        cache_set_json_ex(
            self.state.cache.as_ref(),
            RedisKey::WebdavPendingDir(self.hierarchy_id, &key),
            &set,
            TRANSIENT_TTL_SECS,
        )
        .await
    }

    /// Best-effort: drop the pending marker for `segments` once a real file/tag makes it live.
    async fn clear_pending_dir(&self, segments: &[String]) {
        let Ok((parent, name)) = split_last(segments) else {
            return;
        };
        let key = path_key(parent);
        let Ok(mut set) = self.pending_dir_children(parent).await else {
            return;
        };
        let before = set.len();
        set.retain(|n| n != &name);
        if set.len() == before {
            return;
        }
        let cache = self.state.cache.as_ref();
        let _ = if set.is_empty() {
            cache
                .del(RedisKey::WebdavPendingDir(self.hierarchy_id, &key))
                .await
        } else {
            cache_set_json_ex(
                cache,
                RedisKey::WebdavPendingDir(self.hierarchy_id, &key),
                &set,
                TRANSIENT_TTL_SECS,
            )
            .await
        };
    }

    // ── OS-junk sidecars (§11) ────────────────────────────────────────────────────

    async fn sidecar_map(&self, parent: &[String]) -> Result<HashMap<String, Sidecar>, AppError> {
        let key = path_key(parent);
        Ok(cache_get_json::<HashMap<String, Sidecar>>(
            self.state.cache.as_ref(),
            RedisKey::WebdavSidecar(self.hierarchy_id, &key),
        )
        .await?
        .unwrap_or_default())
    }

    /// All sidecar files stored under `parent`.
    async fn sidecars(&self, parent: &[String]) -> Result<Vec<Sidecar>, AppError> {
        Ok(self.sidecar_map(parent).await?.into_values().collect())
    }

    /// The sidecar at `segments`, if any.
    async fn sidecar(&self, segments: &[String]) -> Result<Option<Sidecar>, AppError> {
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

    /// The scratch namespace recorded under `parent`.
    async fn staging_parent(&self, parent: &[String]) -> Result<StagingParent, AppError> {
        let key = path_key(parent);
        Ok(cache_get_json::<StagingParent>(
            self.state.cache.as_ref(),
            RedisKey::WebdavStaging(self.hierarchy_id, &key),
        )
        .await?
        .unwrap_or_default())
    }

    /// Persist (or drop, when empty) the scratch namespace under `parent`.
    async fn save_staging_parent(
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
    async fn is_staging_dir(&self, segments: &[String]) -> Result<bool, AppError> {
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
    async fn staged_file(&self, segments: &[String]) -> Result<Option<StagedFile>, AppError> {
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
                &self.state.config.s3_bucket_staging,
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
                    .delete_object(&self.state.config.s3_bucket_staging, &key)
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
                .presign_get(&self.state.config.s3_bucket_staging, &key)
                .await?;
            Ok(Some(ReadTarget::Redirect(url)))
        } else {
            let data = self
                .state
                .storage
                .get_object(&self.state.config.s3_bucket_staging, &key)
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
                    .delete_object(&self.state.config.s3_bucket_staging, &key)
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
                        .delete_object(&self.state.config.s3_bucket_staging, key)
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
            )
            .await?;
        if remove_source {
            sp.files.remove(&name);
            self.save_staging_parent(parent, &sp).await?;
            let _ = self
                .state
                .storage
                .delete_object(&self.state.config.s3_bucket_staging, &key)
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

/// Join path segments into the Redis-key path component (slash-delimited; `""` for the root).
fn path_key(segments: &[String]) -> String {
    segments.join("/")
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

/// Project a staged atomic-save scratch file into a file entry for listings (08_webdav_issues.md §1).
fn staged_entry(f: &StagedFile) -> VfsEntry {
    let modified = DateTime::from_timestamp(f.mtime, 0)
        .map(|d| d.naive_utc())
        .unwrap_or_else(|| Utc::now().naive_utc());
    VfsEntry {
        name: f.name.clone(),
        is_dir: false,
        size: f.size.max(0) as u64,
        modified,
        etag: f.hash.clone(),
        mime_type: f.content_type.clone(),
        picture_id: None,
        writable: true,
    }
}

/// Project a stored OS-junk sidecar into a file entry for listings (06_webdav.md §11).
fn sidecar_entry(sc: &Sidecar) -> VfsEntry {
    let modified = DateTime::from_timestamp(sc.mtime, 0)
        .map(|d| d.naive_utc())
        .unwrap_or_else(|| Utc::now().naive_utc());
    VfsEntry {
        name: sc.name.clone(),
        is_dir: false,
        size: sc.size,
        modified,
        etag: None,
        mime_type: sc.content_type.clone(),
        picture_id: None,
        writable: true,
    }
}

/// Split path segments into (parent, last). Errors on empty (no last segment).
fn split_last(segments: &[String]) -> Result<(&[String], String), AppError> {
    match segments.split_last() {
        Some((last, parent)) => Ok((parent, last.clone())),
        None => Err(AppError::NotFound),
    }
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
