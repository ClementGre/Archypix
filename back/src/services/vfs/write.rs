
use crate::domain::hierarchy::{TagOp, TagOpKind};
use crate::domain::tag::TagPath;
use crate::infra::s3;
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::user_settings::UserSettingsRepository;
use crate::services::pictures::{self};
use archypix_common::error::AppError;
use chrono::NaiveDateTime;
use std::path::Path;
use tracing::trace;
use uuid::Uuid;
use super::*;

impl<'a> Vfs<'a> {
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
        original_file_created_at: Option<NaiveDateTime>,
    ) -> Result<bool, AppError> {
        self.finalize_write(
            segments,
            ByteSource::LocalTemp(temp_path),
            hash,
            size,
            content_type,
            original_file_created_at,
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
    pub(super) async fn finalize_write(
        &self,
        segments: &[String],
        source: ByteSource<'_>,
        hash: &str,
        size: i64,
        content_type: Option<&str>,
        original_file_created_at: Option<NaiveDateTime>,
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

                // Quota gate (feature 22 §6): bill the net delta — new bytes minus the replaced
                // original, plus a version snapshot if one is taken. Only a growing overwrite is
                // blocked; a neutral/shrinking one always proceeds.
                {
                    use crate::domain::user_settings::VersioningMode;
                    let old_size = pic.file_size.unwrap_or(0);
                    let will_snapshot = match settings.versioning_mode {
                        VersioningMode::None => false,
                        VersioningMode::OriginalCopy => {
                            !PictureVersionRepository::has_versions(&self.state.db, pic.id).await?
                        }
                        VersioningMode::FullVersioning => true,
                    };
                    let net = size - old_size + if will_snapshot { old_size } else { 0 };
                    if net > 0
                        && !crate::services::storage::fits(
                            self.state.cache.as_ref(),
                            &self.state.db,
                            self.user_id,
                            net,
                        )
                        .await?
                    {
                        return Err(AppError::InsufficientStorage(
                            "overwrite would exceed your storage quota".into(),
                        ));
                    }
                }

                pictures::snapshot_version_on_overwrite(
                    &self.state.db,
                    self.state.storage.as_ref(),
                    &self.state.settings,
                    settings.versioning_mode,
                    &pic,
                )
                .await?;

                let key = s3::picture_key(pic.local_user_id, pic.id);
                source
                    .copy_to(
                        self.state,
                        &self.state.settings.get(keys::S3_BUCKET_PICTURES),
                        &key,
                        content_type,
                    )
                    .await?;
                // Set the new hash/size inline so the ETag is correct before gen_thumbnail re-extracts.
                PictureRepository::set_file_hash(&self.state.db, pid, hash, Some(size)).await?;
                crate::services::storage::invalidate_committed(
                    self.state.cache.as_ref(),
                    self.user_id,
                )
                .await;
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
                // The new bytes are the source of truth until the extraction reads them (33 §6.2).
                PictureRepository::set_exif_sync_status(
                    &self.state.db,
                    pid,
                    crate::domain::picture::ExifSyncStatus::Extracting,
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
            self.bust_tag_tree().await;
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
            self.bust_tag_tree().await;
            self.state.routines.pipeline.trigger_debounced(self.user_id);
            return Ok(true);
        }

        // Quota gate (feature 22 §6): a genuinely new picture bills its full size — reject before
        // writing any bytes when over quota.
        if !crate::services::storage::fits(
            self.state.cache.as_ref(),
            &self.state.db,
            self.user_id,
            size,
        )
        .await?
        {
            return Err(AppError::InsufficientStorage(
                "upload would exceed your storage quota".into(),
            ));
        }

        // Genuine new picture: stream bytes to S3, create the row + thumbnail job, then apply tags.
        let new_id = Uuid::new_v4();
        tracing::Span::current().record("picture_id", tracing::field::display(new_id));
        trace!("vfs put: ingest new picture");
        let key = s3::picture_key(self.user_id, new_id);
        source
            .copy_to(
                self.state,
                &self.state.settings.get(keys::S3_BUCKET_PICTURES),
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
            original_file_created_at,
            crate::services::jobs::ingest_exif_status(content_type),
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
        crate::services::storage::invalidate_committed(self.state.cache.as_ref(), self.user_id)
            .await;
        self.apply_add_ops(&on_add, new_id).await?;
        self.bust_tag_tree().await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(true)
    }

    /// Resolve a (possibly not-yet-existing) directory path to either an existing [`ResolvedDir`]
    /// or a `mirror` extension that mints a deeper tag (06_webdav.md §9). The new trailing
    /// segments must be valid tag labels, and the nearest existing ancestor must be a writable
    /// `mirror` node — otherwise the write is rejected.
    pub(super) fn resolve_path(&self, segments: &[String]) -> Result<PathResolution<'_>, AppError> {
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
    pub(super) fn on_add_ops(&self, target: &PathResolution<'_>) -> Result<Vec<TagOp>, AppError> {
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

}
