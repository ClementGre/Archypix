use crate::backend::BackendClient;
use crate::error::{Result, WorkerError};
use crate::imaging::{content_hash as content_hash_mod, exif as exif_mod, thumbnailer};
use archypix_common::job::EditPictureConfig;
use archypix_common::transfer::{ExifExtraction, PictureWork, PresignedWrites};
use tempfile::TempDir;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Handle an `edit_picture` job.
///
/// Processing order — the modified-original upload is the **last** fallible step, so a permanent
/// failure implies the S3 original was never overwritten (the backend's revert model relies on this
/// file-untouched-on-failure invariant):
/// 0. Preflight: an EXIF-only edit on a format that takes no EXIF writes returns `UnsupportedMime`
///    before the download — a verdict about the MIME needs no bytes (feature 33 §4.3).
/// 1. Download the original file.
/// 2. Apply visual transforms, before the EXIF write — a re-encode strips metadata.
/// 3. If `exif` is set: rewrite the file's embedded EXIF to match the target snapshot, then read it
///    back so the backend can record the physical state (`file_exif`). A file the metadata library
///    cannot open at all is `Failed` (terminal `unsupported_file`); a write that fails on an
///    openable file is a retryable `write_failed`, and a missing tool is retried (feature 31 §6).
///    BMFF images (HEIC/HEIF/AVIF) are written via ExifTool; other formats keep using rexiv2.
/// 4. Regenerate + upload thumbnails (and BlurHash) — a no-op for an EXIF-only edit.
/// 5. Compute file_size and file_hash from the (modified) file.
/// 6. Upload the modified original to the `output` presigned URL — the last fallible step.
#[tracing::instrument(
    skip(client, config, presigned_read, presigned_writes, mime_type),
    fields(job_id = %job_id, picture_id = %config.picture_id),
)]
pub async fn handle(
    client: &BackendClient,
    job_id: Uuid,
    config: EditPictureConfig,
    presigned_read: Option<String>,
    presigned_writes: PresignedWrites,
    mime_type: Option<String>,
    work: &mut PictureWork,
) -> Result<()> {
    let presigned_read = presigned_read.ok_or_else(|| WorkerError::MissingPresignedUrl {
        key: "original".to_string(),
    })?;
    let output_url = presigned_writes
        .output
        .as_deref()
        .ok_or_else(|| WorkerError::MissingPresignedUrl {
            key: "output".to_string(),
        })?
        .to_string();

    // ── Format preflight ──────────────────────────────────────────────────────
    // An EXIF-only edit on a format that takes no EXIF writes has nothing to do. Bail before the
    // download: the verdict comes from the MIME, not the bytes, so fetching them is pure waste —
    // and on a video that is the whole container. An unknown MIME is attempted (feature 33 §10).
    if config.exif.is_some()
        && config.visual.is_none()
        && mime_type
            .as_deref()
            .is_some_and(|m| !archypix_common::mime::supports_exif(m))
    {
        return Err(WorkerError::UnsupportedMime(
            mime_type.unwrap_or_else(|| "unknown".into()),
        ));
    }

    // ── Download ──────────────────────────────────────────────────────────────
    let tmp = TempDir::new()?;
    let file_path = tmp.path().join("original");

    info!("Downloading original");
    client
        .download_presigned(&presigned_read, &file_path)
        .await?;
    debug!(
        size_bytes = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0),
        "Original downloaded"
    );

    // ── Visual transforms ────────────────────────────────────────────────────
    // TODO: implement crop / resize once the imaging primitives are ready.
    // Must stay **above** the EXIF write: a re-encode drops embedded metadata, and a rotate/crop
    // invalidates the `Orientation` tag, so the EXIF target has to be laid down over the result.
    if config.visual.is_some() {
        warn!(job_id = %job_id, "visual transforms not yet implemented; uploading original");
    }

    // ── Apply the EXIF edit (set/clear) into the file ─────────────────────────
    if let Some(ref edit) = config.exif {
        let path = file_path.clone();
        let target = edit.target.clone();
        let mime_type = mime_type.clone();
        let span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            let _guard = span.enter();
            exif_mod::write_exif_target(&path, &target, mime_type.as_deref())
        })
        .await
        .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))??
    }

    // ── Regenerate thumbnails BEFORE the original upload ─────────────────────
    // No-ops unless the backend issued thumbnail presigned URLs, which it does only for a visual
    // edit (`PresignedWrites::exif_only` otherwise) — so an EXIF-only edit never decodes the image.
    // Keeping the original upload last preserves the file-untouched-on-failure invariant.
    let thumb = thumbnailer::run(client, &file_path, &presigned_writes, tmp.path()).await?;
    work.blurhash = thumb.blurhash;
    work.thumbnails_generated = thumb.generated;
    work.width = thumb.width;
    work.height = thumb.height;

    // ── File size + hash (after EXIF write, so values match what is uploaded) ─
    work.file_size = std::fs::metadata(&file_path).map(|m| m.len() as i64).ok();

    let path_for_hash = file_path.clone();
    work.file_hash =
        tokio::task::spawn_blocking(move || archypix_common::hash::hash_file(&path_for_hash))
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
    if work.file_hash.is_none() {
        warn!(job_id = %job_id, "failed to compute file hash; skipping");
    }

    // Metadata-stripped content hash (feature 11): unchanged by an EXIF-only edit, refreshed by a
    // visual re-encode so the dedup reconciler regroups it. Enters the current (job) span like the
    // EXIF-write spawn_blocking task above.
    let path_for_content = file_path.clone();
    let content_span = tracing::Span::current();
    work.content_hash = tokio::task::spawn_blocking(move || {
        let _guard = content_span.enter();
        content_hash_mod::content_hash(&path_for_content)
    })
    .await
    .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;

    // Read back the physical file EXIF after the write so the backend can record `file_exif`.
    work.exif = if config.exif.is_some() {
        let path = file_path.clone();
        let mime = mime_type.clone();
        let span = tracing::Span::current();
        ExifExtraction::Extracted(
            tokio::task::spawn_blocking(move || {
                let _guard = span.enter();
                exif_mod::read_metadata(&path, mime.as_deref())
            })
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?
            // The write already succeeded, so this is not a format verdict: downgrade a terminal
            // `unsupported` to a retryable write failure (feature 31 §6).
            .map_err(|e| match e {
                WorkerError::UnsupportedFormat(m) => {
                    WorkerError::Exif(format!("EXIF read-back after write failed: {m}"))
                }
                other => other,
            })?,
        )
    } else {
        ExifExtraction::NotAttempted
    };

    // ── Upload modified original (last fallible step) ────────────────────────
    info!(job_id = %job_id, "edit_picture: uploading modified original");
    client.upload_presigned(&output_url, &file_path).await?;

    info!(
        job_id = %job_id,
        thumbnails_regenerated = work.thumbnails_generated,
        "edit_picture completed"
    );
    Ok(())
}
