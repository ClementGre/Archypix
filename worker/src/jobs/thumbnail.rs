//! Handler for `gen_thumbnail` jobs.
//!
//! Sequence: download → file_size + file_hash → EXIF extraction (initial only)
//! → thumbnail generation (only when the MIME supports it) → upload → complete.
//!
//! `file_size`/`file_hash` are computed **before** the thumbnail-support decision so that a
//! format we cannot thumbnail (e.g. a RAW/heic without a codec) still reports its size and hash
//! and completes successfully — only the thumbnails are skipped. This is what lets the backend
//! hold a correct ETag/size for every successfully-ingested picture, not just thumbnailable ones.
//!
//! Error policy:
//! - Unsupported MIME for thumbnailing  → complete without thumbnails (size/hash still reported)
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

#[tracing::instrument(
    skip(client, config, presigned_read, presigned_writes),
    fields(job_id = %job_id, picture_id = %config.picture_id),
)]
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

    // ── MIME pre-flight (decides what work to do, not whether to fail) ────────
    // A non-thumbnailable format is no longer an error: we still download, size, and hash it,
    // then complete with thumbnails skipped. Only generate thumbnails when the job asked for them
    // *and* the codec supports it.
    let do_thumbnails = presigned_writes.has_thumbnails()
        && mime_type
            .as_deref()
            .map(archypix_common::mime::supports_thumbnail)
            .unwrap_or(true);
    if !do_thumbnails && presigned_writes.has_thumbnails() {
        warn!(mime_type = ?mime_type, "gen_thumbnail: MIME not thumbnailable; reporting size/hash only");
    }
    if let Some(ref mime) = mime_type {
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

    info!("Downloading original...");
    client
        .download_presigned(&presigned_read, &original_path)
        .await?;

    let file_size = std::fs::metadata(&original_path)
        .map(|m| m.len() as i64)
        .ok();
    debug!(size_bytes = ?file_size, "Original downloaded");

    // ── EXIF extraction (initial jobs only, blocking) ─────────────────────────
    let exif: Option<ExtractedExif> = if should_extract_exif {
        let path = original_path.clone();
        let span = tracing::info_span!("exif_extract", file = ?path.file_name());
        match tokio::task::spawn_blocking(move || {
            let _guard = span.enter();
            exif_mod::extract_exif(&path)
        })
        .await
        .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?
        {
            Ok(e) => {
                debug!("EXIF extracted");
                Some(e)
            }
            Err(e) => {
                warn!(error = ?e, "EXIF extraction failed; continuing without EXIF");
                None
            }
        }
    } else {
        None
    };

    // ── File hash (blocking) ─────────────────────────────────────────────────
    let path_for_hash = original_path.clone();
    info!(path = %path_for_hash.display(), "Hashing file...");
    let file_hash =
        tokio::task::spawn_blocking(move || archypix_common::hash::hash_file(&path_for_hash))
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
    if file_hash.is_none() {
        warn!("File hash failed; skipping");
    }

    // ── Thumbnails + BlurHash + upload (skipped for non-thumbnailable formats) ─
    let (blurhash, thumbnails_generated, decoded_dims) = if do_thumbnails {
        let thumb = thumbnailer::run(client, &original_path, &presigned_writes, tmp.path()).await?;
        (thumb.blurhash, thumb.generated, (thumb.width, thumb.height))
    } else {
        (None, false, (None, None))
    };

    // Dimensions: prefer the decoded image (authoritative, orientation-consistent with the raw
    // thumbnails); fall back to EXIF only when the image was not decoded (non-thumbnailable format).
    let exif_dims = exif
        .as_ref()
        .map(|e| (e.width, e.height))
        .unwrap_or((None, None));
    let width = decoded_dims.0.or(exif_dims.0);
    let height = decoded_dims.1.or(exif_dims.1);

    client
        .complete_job(
            job_id,
            CompleteJobRequest {
                claim_token,
                exif,
                blurhash,
                thumbnails_generated,
                file_size,
                file_hash,
                width,
                height,
            },
        )
        .await?;

    Ok(())
}
