//! The protocol-agnostic virtual filesystem over a hierarchy (06_webdav.md §5).
//!
//! [`Vfs`] resolves a hierarchy into a directory tree once (via
//! [`services::hierarchy::load_resolved`]) and exposes filesystem operations — list/stat/read
//! and the write taxonomy (PUT/DELETE/MOVE/COPY/MKCOL) — as tag mutations and uploads. The
//! WebDAV adapter in `api::webdav` is a thin shell over this; an SFTP adapter could reuse it.

use crate::domain::hierarchy::{NamingStrategy, TagOp, TagOpKind};
use crate::domain::tag_metadata::{
    TagMetadataPatch, validate_display_name, validate_webdav_dir_name,
};
use crate::infra::settings::keys;
use crate::services::hierarchy::ResolvedDir;
use crate::state::AppState;
use archypix_common::error::AppError;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
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
                    .copy_object(
                        &state.settings.get(keys::S3_BUCKET_STAGING),
                        key,
                        dst_bucket,
                        dst_key,
                    )
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

mod dirops;
mod read;
mod sidecar;
mod staging;
mod write;

/// Join path segments into the Redis-key path component (slash-delimited; `""` for the root).
fn path_key(segments: &[String]) -> String {
    segments.join("/")
}

/// The patch that makes `tag` a directory named `requested_name` (§8.1): the name is kept verbatim
/// as `webdav_dir_name` and as the display name, falling back to the ltree label when a validator
/// rejects it (an over-long or control-laden folder name still makes a directory).
fn named_empty_dir_patch(tag: &str, requested_name: &str) -> TagMetadataPatch {
    TagMetadataPatch {
        tag_path: tag.to_string(),
        display_name: Some(validate_display_name(requested_name).ok()),
        webdav_dir_name: Some(validate_webdav_dir_name(requested_name).ok()),
        show_when_empty: Some(true),
        ..Default::default()
    }
}

/// Every tag a resolved tree maps a directory to — including the empty ones a `tags` query cannot
/// see (34 §8.1).
fn collect_mirror_tags(dir: &ResolvedDir, out: &mut Vec<String>) {
    if let Some(tag) = &dir.mirror_tag {
        out.push(tag.clone());
    }
    for child in &dir.children {
        collect_mirror_tags(child, out);
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
                // Bytes-only mtime (feature 32) — `updated_at` moves on every re-tag, which makes
                // mtime-comparing sync clients re-download the whole library.
                modified: p.file_modified_at,
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
