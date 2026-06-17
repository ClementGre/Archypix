//! Handler for `gen_thumbnail` jobs.
//!
//! Sequence: MIME pre-flight → download → EXIF extraction (initial only)
//! → file_size + file_hash → thumbnail generation → upload → complete.
//!
//! Error policy:
//! - Unsupported MIME for thumbnailing  → `WorkerError::UnsupportedFormat` (permanent)
//! - Image codec failure                → `WorkerError::Imaging` (permanent)
//! - EXIF extraction failure            → log and continue (EXIF is optional)
//! - BlurHash failure                   → log and continue (nice-to-have)
//! - Network / upload failure           → propagated `WorkerError::Http` (retriable)

use crate::backend::BackendClient;
use crate::error::{Result, WorkerError};
use crate::imaging::{exif as exif_mod, thumbnailer};
use archypix_common::job::{ExtractedExif, GenThumbnailConfig};
use archypix_common::transfer::{CompleteJobRequest, PresignedWrites};
use tempfile::TempDir;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub async fn handle(
    client: &BackendClient,
    job_id: Uuid,
    claim_token: Uuid,
    config: GenThumbnailConfig,
    presigned_read: Option<String>,
    presigned_writes: PresignedWrites,
    mime_type: Option<String>,
) -> Result<()> {
    let presigned_read = presigned_read.ok_or_else(|| WorkerError::MissingPresignedUrl {
        key: "original".to_string(),
    })?;

    // ── MIME pre-flight (before downloading) ─────────────────────────────────
    if let Some(ref mime) = mime_type {
        if presigned_writes.has_thumbnails() && !archypix_common::mime::supports_thumbnail(mime) {
            return Err(WorkerError::UnsupportedFormat(format!(
                "MIME type '{mime}' is not supported for thumbnail generation"
            )));
        }
        if config.is_initial && !archypix_common::mime::supports_exif(mime) {
            warn!(mime_type = %mime, "MIME type not supported for EXIF extraction; skipping");
        }
    }

    let should_extract_exif = config.is_initial
        && mime_type
            .as_deref()
            .map(archypix_common::mime::supports_exif)
            .unwrap_or(true);

    // ── Download ──────────────────────────────────────────────────────────────
    let tmp = TempDir::new()?;
    let original_path = tmp.path().join("original");

    info!(job_id = %job_id, "gen_thumbnail: downloading original");
    client
        .download_presigned(&presigned_read, &original_path)
        .await?;

    let file_size = std::fs::metadata(&original_path)
        .map(|m| m.len() as i64)
        .ok();
    debug!(size_bytes = ?file_size, "gen_thumbnail: original downloaded");

    // ── EXIF extraction (initial jobs only, blocking) ─────────────────────────
    let exif: Option<ExtractedExif> = if should_extract_exif {
        let path = original_path.clone();
        match tokio::task::spawn_blocking(move || exif_mod::extract_exif(&path))
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?
        {
            Ok(e) => {
                debug!("gen_thumbnail: EXIF extracted");
                Some(e)
            }
            Err(e) => {
                warn!(error = ?e, "gen_thumbnail: EXIF extraction failed; continuing without EXIF");
                None
            }
        }
    } else {
        None
    };

    // ── File hash (blocking) ─────────────────────────────────────────────────
    let path_for_hash = original_path.clone();
    let file_hash =
        tokio::task::spawn_blocking(move || archypix_common::hash::hash_file(&path_for_hash))
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
    if file_hash.is_none() {
        warn!("gen_thumbnail: file hash failed; skipping");
    }

    // ── Thumbnails + BlurHash + upload ────────────────────────────────────────
    let thumb = thumbnailer::run(client, &original_path, &presigned_writes, tmp.path()).await?;

    client
        .complete_job(
            job_id,
            CompleteJobRequest {
                claim_token,
                exif,
                blurhash: thumb.blurhash,
                thumbnails_generated: thumb.generated,
                file_size,
                file_hash,
            },
        )
        .await?;

    info!(job_id = %job_id, thumbnails_generated = thumb.generated, "gen_thumbnail completed");
    Ok(())
}
