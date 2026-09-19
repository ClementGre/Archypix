
use crate::domain::hierarchy::{SafeDeleteMode, TagOp};
use crate::domain::tag_metadata::TagMetadataPatch;
use crate::repository::picture::PictureRepository;
use crate::repository::tag::TagRepository;
use crate::repository::tag_metadata::TagMetadataRepository;
use crate::services::hierarchy::ResolvedDir;
use archypix_common::error::AppError;
use tracing::trace;
use uuid::Uuid;
use super::*;

impl<'a> Vfs<'a> {
    /// DELETE a file per the directory's `safeDeleteMode` (§7.1).
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/"), picture_id))]
    pub async fn delete(&self, segments: &[String]) -> Result<(), AppError> {
        // Deleting a real directory: accept it if the directory is empty. An empty one may be a
        // `MKCOL`'d `show_when_empty` tag, whose metadata row *is* the directory (§8.1).
        if let Some(dir) = self.dir(segments) {
            if self.list_dir(segments).await?.is_empty() {
                if let Some(tag) = dir.mirror_tag.clone() {
                    self.delete_tag_metadata(&tag).await?;
                }
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
        self.bust_tag_tree().await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(())
    }

    /// MOVE: rename within a directory, or re-file across directories (§7.1).
    #[tracing::instrument(
        skip(self),
        fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, from = %from.join("/"), to = %to.join("/"), picture_id)
    )]
    pub async fn move_(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        if self.dir(from).is_some() {
            return self.move_empty_dir(from, to).await;
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
        self.bust_tag_tree().await;
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
        self.bust_tag_tree().await;
        self.state.routines.pipeline.trigger_debounced(self.user_id);
        Ok(())
    }

    /// MKCOL: directories are tag-derived. Under a writable `mirror` node a brand-new sub-path
    /// mints a `show_when_empty` tag-metadata row, so the directory persists and lists with no
    /// pictures in it (34_tag_metadata.md §8.1). The requested name is kept verbatim as
    /// `webdav_dir_name` (and as the display name), with the slugified label as the tag itself.
    /// `static`/`query` structure is fixed, and an already-existing path is rejected.
    #[tracing::instrument(skip(self), fields(user_id = %self.user_id, hierarchy_id = %self.hierarchy_id, path = %segments.join("/")))]
    pub async fn mkcol(&self, segments: &[String]) -> Result<(), AppError> {
        let (parent, requested) = split_last(segments)?;
        // A drop inbox is a leaf — MKCOL inside it is not allowed (feature 18 §4).
        if self.dir(parent).is_some_and(|d| d.always_visible) {
            return Err(AppError::MethodNotAllowed(
                "cannot create a directory inside a drop inbox".into(),
            ));
        }
        let tag = self.resolve_new_tag(segments).await?;
        trace!(%tag, "vfs mkcol: minting an empty tag");
        self.create_empty_tag(&tag, &requested).await
    }

    /// Whether any directory in the resolved tree already maps to `tag` — the slugified label of a
    /// `MKCOL` can collide with an existing sibling whose displayed name differs.
    pub(super) fn tag_dir_exists(&self, tag: &str) -> bool {
        fn walk(dir: &ResolvedDir, tag: &str) -> bool {
            dir.mirror_tag.as_deref() == Some(tag) || dir.children.iter().any(|c| walk(c, tag))
        }
        walk(&self.root, tag)
    }

    /// MOVE on a collection is out of scope in general (99_ROADMAP "Advanced WebDAV") — with one
    /// exception: a still-empty `MKCOL`'d directory, which is Finder's create-then-rename flow
    /// (it creates `untitled folder` and MOVEs it to the typed name). Anything else is `405`.
    pub(super) async fn move_empty_dir(&self, from: &[String], to: &[String]) -> Result<(), AppError> {
        let unsupported = || {
            AppError::MethodNotAllowed(
                "renaming or moving a directory is not supported — rename the tag instead".into(),
            )
        };
        let (from_parent, _) = split_last(from)?;
        let (to_parent, requested) = split_last(to)?;
        // Reparenting is a tag rename, not a folder rename.
        if from_parent != to_parent {
            return Err(unsupported());
        }
        let dir = self.dir(from).ok_or(AppError::NotFound)?;
        let Some(old_tag) = dir.mirror_tag.clone() else {
            return Err(unsupported());
        };
        if !dir.writable || !self.list_dir(from).await?.is_empty() {
            return Err(unsupported());
        }
        // Only a directory the metadata row itself conjures may be renamed this way (§8.1).
        if !self.is_show_when_empty(&old_tag).await? {
            return Err(unsupported());
        }

        let new_tag = self.resolve_new_tag(to).await?;
        // A tag rename is only safe when nothing at all hangs off the old path — a *trashed*
        // picture still carries it and never shows in a listing, and re-filing those belongs to
        // the tag-rename cascade. Otherwise mint a fresh tag and retire the source directory.
        if TagRepository::subtree_has_pictures(&self.state.db, self.user_id, &old_tag, true).await?
        {
            trace!(%old_tag, %new_tag, "vfs move: source tag is not bare — minting a new one");
            self.create_empty_tag(&new_tag, &requested).await?;
            return self
                .patch_tag_metadata(TagMetadataPatch {
                    tag_path: old_tag,
                    show_when_empty: Some(false),
                    ..Default::default()
                })
                .await;
        }
        trace!(%old_tag, %new_tag, "vfs move: renaming a bare empty directory");
        TagMetadataRepository::rename_subtree(&self.state.db, self.user_id, &old_tag, &new_tag)
            .await?;
        // The folder keeps the name the client typed (§8).
        self.patch_tag_metadata(named_empty_dir_patch(&new_tag, &requested))
            .await
    }

    /// The tag a not-yet-existing directory path would mint, folded onto an existing case variant
    /// and rejected if anything already occupies it (§8.1, §10c).
    pub(super) async fn resolve_new_tag(&self, segments: &[String]) -> Result<String, AppError> {
        let conflict = || AppError::Conflict("directory already exists".into());
        if self.dir(segments).is_some() {
            return Err(conflict());
        }
        let PathResolution::MirrorExtension { tag } = self.resolve_path(segments)? else {
            return Err(conflict());
        };
        // Fold onto an existing case-variant sibling, as the PUT path does (§10c) — a tag that
        // already exists under a different display name is still a conflict.
        let tag = self.fold_case(vec![tag]).await?.remove(0);
        if self.tag_dir_exists(&tag) {
            return Err(conflict());
        }
        Ok(tag)
    }

    /// Mint the `show_when_empty` metadata row that *is* a brand-new empty directory (§8.1).
    pub(super) async fn create_empty_tag(&self, tag: &str, requested_name: &str) -> Result<(), AppError> {
        self.patch_tag_metadata(named_empty_dir_patch(tag, requested_name))
            .await
    }

    /// Whether `tag`'s row is what makes its directory exist.
    pub(super) async fn is_show_when_empty(&self, tag: &str) -> Result<bool, AppError> {
        Ok(
            TagMetadataRepository::find_many(&self.state.db, self.user_id, &[tag.to_string()])
                .await?
                .first()
                .is_some_and(|m| m.show_when_empty),
        )
    }

    /// Drop the row behind an empty `MKCOL`'d directory (§8.1) — only when `show_when_empty` is
    /// what made the directory exist. A directory can also be empty because the hierarchy filtered
    /// every one of its pictures out (a foreign `exclude`, feature 18 §7.3), and wiping a live
    /// tag's decoration for that would be silent, undoable data loss.
    pub(super) async fn delete_tag_metadata(&self, tag: &str) -> Result<(), AppError> {
        if !self.is_show_when_empty(tag).await? {
            trace!(%tag, "vfs delete: empty directory is tag-derived — nothing to drop");
            return Ok(());
        }
        if TagRepository::subtree_has_pictures(&self.state.db, self.user_id, tag, true).await? {
            trace!(%tag, "vfs delete: tag still has pictures — keeping its metadata");
            return Ok(());
        }
        trace!(%tag, "vfs delete: empty directory — drop its metadata row");
        crate::services::tag_metadata::delete(
            &self.state.db,
            self.state.cache.as_ref(),
            self.user_id,
            &[tag.to_string()],
        )
        .await?;
        Ok(())
    }

    /// WebDAV mints tags outside the API, so metadata writes go through the same service as the
    /// API's — merge-onto-stored, the prune-if-all-default rule and the cache bust included.
    pub(super) async fn patch_tag_metadata(&self, patch: TagMetadataPatch) -> Result<(), AppError> {
        crate::services::tag_metadata::upsert(
            &self.state.db,
            self.state.cache.as_ref(),
            self.user_id,
            vec![patch],
        )
        .await?;
        Ok(())
    }

    /// WebDAV mints tags outside the API, so the cached tag tree has to be dropped here too
    /// (34_tag_metadata.md §4 leaves the *read* path to the TTL, but a write we perform is known).
    pub(super) async fn bust_tag_tree(&self) {
        crate::services::tag_metadata::bust_cache(self.state.cache.as_ref(), self.user_id).await;
    }

    // `user_id`/`picture_id` are already on the calling span (put_file/move_/copy record
    // `picture_id` before reaching here) — no fields of our own to add.
    /// Returns whether the picture actually gained a tag (≥1 row inserted)
    #[tracing::instrument(skip_all)]
    pub(super) async fn apply_add_ops(&self, ops: &[TagOp], pid: Uuid) -> Result<bool, AppError> {
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
    pub(super) async fn fold_case(&self, assigns: Vec<String>) -> Result<Vec<String>, AppError> {
        if assigns.is_empty() {
            return Ok(assigns);
        }
        let mut existing = TagRepository::list_paths_by_user(&self.state.db, self.user_id).await?;
        // That query joins `pictures`, so a `show_when_empty` tag is invisible to it; the resolved
        // tree already carries them (34 §8.1) and they must fold like any other sibling, or MKCOL
        // mints a case-variant directory beside one it should have reused.
        collect_mirror_tags(&self.root, &mut existing);
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
    pub(super) async fn apply_remove_ops(&self, ops: &[TagOp], pid: Uuid) -> Result<(), AppError> {
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

    // ── OS-junk sidecars (§11) ────────────────────────────────────────────────────

}
