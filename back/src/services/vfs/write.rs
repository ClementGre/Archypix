
use crate::domain::hierarchy::TagOp;
use crate::domain::tag::TagPath;
use crate::infra::s3;
use crate::infra::settings::keys;
use crate::repository::picture::PictureRepository;
use crate::repository::picture_version::PictureVersionRepository;
use crate::repository::user_settings::UserSettingsRepository;
use crate::services::hierarchy::{ChainNode, node_chain, transition};
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
        let file = NewFile {
            source: ByteSource::LocalTemp(temp_path),
            hash,
            size,
            content_type,
            created_at: original_file_created_at,
        };
        self.finalize_write(segments, file).await
    }

    /// Finalize a write to `segments` from either a local temp file or a staged object
    /// (08_webdav_issues.md §1.6): overwrite an existing picture (versioned), dedupe/relocate on a
    /// hash hit, or ingest a genuinely new picture and apply the target's tags.
    #[tracing::instrument(
        skip(self, file),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), hash = %file.hash, bytes = file.size, picture_id)
    )]
    pub(super) async fn finalize_write(
        &self,
        segments: &[String],
        file: NewFile<'_>,
    ) -> Result<bool, AppError> {
        let (parent, name) = split_last(segments)?;
        let (hash, size, content_type) = (file.hash, file.size, file.content_type);

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

        // Overwrite is only possible inside an existing directory — not a drop inbox, whose listing
        // is for verification only: a same-named upload there is a new photo (feature 39 §7).
        if let PathResolution::Existing(dir) = &target
            && !dir.is_drop
        {
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
                file.source
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

        // New file — every onAdd of the destination's node chain (feature 39 §3.2).
        let (_, enter) = transition(&[], &self.writable_chain(parent, &target)?);
        let (pid, created) = self.land_new(&target, &enter, &file, &name).await?;
        if let Some(dir) = target.drop_dir() {
            self.note_drop(dir, &name, pid).await;
        }
        self.bust_tag_tree().await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(created)
    }

    /// Land a new file: retag a live duplicate, apply `trashedDuplicate` to a trashed one, or ingest
    /// it. Returns the picture and whether a new resource was created for this path.
    async fn land_new(
        &self,
        target: &PathResolution<'_>,
        enter: &[TagOp],
        file: &NewFile<'_>,
        name: &str,
    ) -> Result<(Uuid, bool), AppError> {
        let (hash, size, content_type) = (file.hash, file.size, file.content_type);
        // Dedupe: a relocate/copy a dumb client expressed as a fresh upload (§8). If the picture
        // gains the directory's tag (it wasn't already here) it's a genuine new resource for this
        // path → 201 Created; if it already had the tag the PUT is a true no-op → 204 No Content.
        if let Some(p) =
            PictureRepository::find_owned_by_hash(&self.state.db, self.user_id, hash, false).await?
        {
            tracing::Span::current().record("picture_id", tracing::field::display(p.id));
            trace!("vfs put: hash matched live picture — retag instead of new upload");
            let added = self.apply_transition(p.id, &[], enter, false).await?;
            return Ok((p.id, added));
        }
        // A trashed match: a naive rename under fullDelete (restore), or a deleted photo a sync
        // client re-sends (ignore) — feature 39 §6.
        if let Some(p) =
            PictureRepository::find_owned_by_hash(&self.state.db, self.user_id, hash, true).await?
        {
            tracing::Span::current().record("picture_id", tracing::field::display(p.id));
            match target.trashed_duplicate() {
                TrashedDuplicate::Restore => {
                    trace!("vfs put: hash matched trashed picture — restore and retag");
                    self.apply_transition(p.id, &[], enter, true).await?;
                }
                TrashedDuplicate::Ignore => {
                    trace!("vfs put: hash matched trashed picture — the delete stands");
                }
            }
            return Ok((p.id, true));
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
        file.source
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
            Some(name),
            content_type,
            Some(size),
            None,
            None,
            None,
            None,
            file.created_at,
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
        self.apply_transition(new_id, &[], enter, false).await?;
        Ok((new_id, true))
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
            let Some(base) = anc.extension_base() else {
                return Err(AppError::Forbidden(
                    "cannot create directories outside a mirror node".into(),
                ));
            };
            if !base.writable {
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
            let candidate = format!("{}.{labels}", base.tag);
            // Slugified labels are valid; `parse` still rejects a reserved (`SharedToMe`) prefix.
            let tag = TagPath::parse(&candidate, false)
                .map_err(AppError::Conflict)?
                .as_ltree()
                .to_string();
            return Ok(PathResolution::MirrorExtension {
                tag,
                base_len: i,
                base,
            });
        }
        Err(AppError::NotFound)
    }

    /// The node chain a write enters (feature 39 §3.1); `403` on a read-only directory (a mirror
    /// extension's writability was checked on resolution). An extension takes its mirror's step.
    pub(super) fn writable_chain(
        &self,
        segments: &[String],
        target: &PathResolution<'_>,
    ) -> Result<Vec<ChainNode>, AppError> {
        match target {
            PathResolution::Existing(dir) if dir.write_back.is_none() => {
                Err(AppError::Forbidden("directory is read-only".into()))
            }
            PathResolution::Existing(_) => Ok(node_chain(&self.root, segments).unwrap_or_default()),
            PathResolution::MirrorExtension {
                tag,
                base_len,
                base,
            } => {
                let parent = &segments[..*base_len];
                let mut chain = node_chain(&self.root, parent).unwrap_or_default();
                // Under the mirror's own directory, the new tag replaces that directory's step.
                if self.dir(parent).and_then(|d| d.node_id.as_deref()) == Some(&base.node_id) {
                    chain.pop();
                }
                chain.push(base.chain_node(tag));
                Ok(chain)
            }
        }
    }
}
