//! Handler for `gen_thumbnail` jobs.
//!
//! Sequence: download → file_size + file_hash → metadata extraction (initial only) → thumbnail
//! generation (only when the MIME supports it) → upload → complete. Images use GExiv2 EXIF +
//! ImageMagick thumbnails; videos use ffprobe metadata + an ffmpeg frame-grab fed to the same WebP
//! thumbnail pipeline (`imaging::video`).
//!
//! `file_size`/`file_hash` are computed **before** the thumbnail-support decision so that a
//! format we cannot thumbnail (e.g. a RAW/heic without a codec) still reports its size and hash
//! and completes successfully — only the thumbnails are skipped. This is what lets the backend
//! hold a correct ETag/size for every successfully-ingested picture, not just thumbnailable ones.
//!
//! Error policy:
//! - Unsupported MIME for thumbnailing  → complete without thumbnails (size/hash still reported)
//! - Image codec failure                → `WorkerError::Imaging` (permanent)
//! - EXIF extraction failure            → a reported outcome, never a silent skip (feature 33 §6.1):
//!   a permanent one completes as `Failed`/`UnsupportedMime`, a retriable one fails the job
//! - BlurHash failure                   → log and continue (nice-to-have)
//! - Network / upload failure           → propagated `WorkerError::Http` (retriable)

use crate::backend::BackendClient;
use crate::error::{Result, WorkerError};
use crate::imaging::{
    content_hash as content_hash_mod, exif as exif_mod, thumbnailer, video as video_mod,
};
use archypix_common::job::GenThumbnailConfig;
use archypix_common::mime::{supports_exif, supports_image_thumbnail, supports_video};
use archypix_common::transfer::{ExifExtraction, PictureWork, PresignedWrites};
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
    config: GenThumbnailConfig,
    presigned_read: Option<String>,
    presigned_writes: PresignedWrites,
    mime_type: Option<String>,
    work: &mut PictureWork,
) -> Result<()> {
    let presigned_read = presigned_read.ok_or_else(|| WorkerError::MissingPresignedUrl {
        key: "original".to_string(),
    })?;

    // ── MIME pre-flight (decides which path to take, not whether to fail) ─────
    // Three disjoint cases: image (GExiv2 EXIF + ImageMagick thumbnail), video (ffprobe metadata +
    // ffmpeg frame-grab thumbnail), or neither (still download/size/hash, skip thumbnails). An
    // unknown MIME defaults to the image path. A format we cannot thumbnail is not an error.
    let is_video = mime_type.as_deref().map(supports_video).unwrap_or(false);
    let is_image_thumbnailable = mime_type
        .as_deref()
        .map(supports_image_thumbnail)
        .unwrap_or(!is_video);
    let want_thumbnails = presigned_writes.has_thumbnails();
    if want_thumbnails && !is_video && !is_image_thumbnailable {
        warn!(mime_type = ?mime_type, "gen_thumbnail: MIME not thumbnailable; reporting size/hash only");
    }

    // An unknown MIME is attempted anyway (feature 33 §4.3): the read resolves it to a real verdict.
    let extract_image_exif =
        config.is_initial && !is_video && mime_type.as_deref().map(supports_exif).unwrap_or(true);

    // ── Download ──────────────────────────────────────────────────────────────
    let tmp = TempDir::new()?;
    let original_path = tmp.path().join("original");

    info!("Downloading original...");
    client
        .download_presigned(&presigned_read, &original_path)
        .await?;

    work.file_size = std::fs::metadata(&original_path)
        .map(|m| m.len() as i64)
        .ok();
    debug!(size_bytes = ?work.file_size, "Original downloaded");

    // ── Metadata extraction (initial jobs only, blocking) ─────────────────────
    // Runs before the thumbnails/hashes so a retriable failure costs one download (feature 33 §6.1).
    // Image EXIF via the two-engine read path, or video container metadata via ffprobe; both map
    // onto ExtractedExif. The outcome — not a silent skip — is what settles the picture's state.
    // Written into the caller's product so a failure *after* this point still reports it (33 §6.5).
    work.exif = if extract_image_exif || (config.is_initial && is_video) {
        let path = original_path.clone();
        let mime = mime_type.clone();
        let span =
            tracing::info_span!("metadata_extract", file = ?path.file_name(), video = is_video);
        let result = tokio::task::spawn_blocking(move || {
            let _guard = span.enter();
            if is_video {
                video_mod::extract_video_metadata(&path)
            } else {
                exif_mod::read_metadata(&path, mime.as_deref())
            }
        })
        .await
        .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
        match result {
            Ok(e) => {
                debug!("metadata extracted");
                ExifExtraction::Extracted(e)
            }
            // Retriable (IO, tool outage) → fail the job; the retry budget and watchdog take over.
            Err(e) if e.is_retriable() => return Err(e),
            Err(e) => {
                warn!(error = ?e, "no engine could read this file's metadata");
                ExifExtraction::Failed
            }
        }
    } else if config.is_initial {
        warn!(mime_type = ?mime_type, "MIME type carries no readable metadata");
        ExifExtraction::UnsupportedMime
    } else {
        ExifExtraction::NotAttempted
    };

    // ── File hash (blocking) ─────────────────────────────────────────────────
    let path_for_hash = original_path.clone();
    info!(path = %path_for_hash.display(), "Hashing file...");
    work.file_hash =
        tokio::task::spawn_blocking(move || archypix_common::hash::hash_file(&path_for_hash))
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
    if work.file_hash.is_none() {
        warn!("File hash failed; skipping");
    }

    // ── Content hash (metadata-stripped, blocking) ───────────────────────────
    // Drives content dedup (feature 11). `None` for a format we can't strip — the backend then
    // groups by `file_hash` instead. The blocking task enters the current (job) span so its work is
    // attributed to the job trace, like the EXIF/edit spawn_blocking tasks.
    let path_for_content = original_path.clone();
    let content_span = tracing::Span::current();
    work.content_hash = tokio::task::spawn_blocking(move || {
        let _guard = content_span.enter();
        content_hash_mod::content_hash(&path_for_content)
    })
    .await
    .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;

    // ── Thumbnails + BlurHash + upload (skipped for non-thumbnailable formats) ─
    // Images thumbnail the original directly; videos thumbnail an extracted frame. A failed
    // frame-grab is non-fatal — we complete with thumbnails skipped (like a non-thumbnailable
    // format) so the picture still reports size/hash.
    let thumb_source: Option<std::path::PathBuf> = if want_thumbnails && is_image_thumbnailable {
        Some(original_path.clone())
    } else if want_thumbnails && is_video {
        let frame = tmp.path().join("frame.png");
        let src = original_path.clone();
        let dst = frame.clone();
        let span = tracing::info_span!("video_frame", file = ?src.file_name());
        let grab = tokio::task::spawn_blocking(move || {
            let _guard = span.enter();
            video_mod::extract_frame(&src, &dst)
        })
        .await
        .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
        match grab {
            Ok(()) => Some(frame),
            Err(e) => {
                warn!(error = ?e, "video frame extraction failed; skipping thumbnails");
                None
            }
        }
    } else {
        None
    };

    let decoded_dims = if let Some(ref src) = thumb_source {
        let thumb = thumbnailer::run(client, src, &presigned_writes, tmp.path()).await?;
        work.blurhash = thumb.blurhash;
        work.thumbnails_generated = thumb.generated;
        (thumb.width, thumb.height)
    } else {
        (None, None)
    };

    // Dimensions: prefer the decoded image (authoritative, orientation-consistent with the raw
    // thumbnails); fall back to EXIF only when the image was not decoded (non-thumbnailable format).
    let exif_dims = work
        .exif
        .extracted()
        .map(|e| (e.width, e.height))
        .unwrap_or((None, None));
    work.width = decoded_dims.0.or(exif_dims.0);
    work.height = decoded_dims.1.or(exif_dims.1);

    Ok(())
}
