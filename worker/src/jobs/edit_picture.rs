use crate::backend::BackendClient;
use crate::error::{Result, WorkerError};
use crate::imaging::{content_hash as content_hash_mod, exif as exif_mod, thumbnailer};
use archypix_common::job::EditPictureConfig;
use archypix_common::transfer::{CompleteJobRequest, PresignedWrites};
use tempfile::TempDir;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Handle an `edit_picture` job.
///
/// Processing order — the modified-original upload is the **last** fallible step, so a permanent
/// failure implies the S3 original was never overwritten (the backend's revert model relies on this
/// file-untouched-on-failure invariant):
/// 1. Download the original file.
/// 2. If `exif` is set: apply its `set`/`clear` delta into the file's embedded EXIF. A write failure
///    is permanent — the backend's MIME preflight prevents enqueuing doomed jobs, so a failure here
///    is genuinely unrecoverable. BMFF images (HEIC/HEIF/AVIF) are written via ExifTool; other image
///    formats keep using rexiv2.
/// 3. Regenerate + upload thumbnails (and BlurHash) from the local edited file (visual edits only).
/// 4. Compute file_size and file_hash from the (modified) file.
/// 5. Upload the modified original to the `output` presigned URL — the last fallible step.
#[tracing::instrument(
    skip(client, config, presigned_read, presigned_writes, mime_type),
    fields(job_id = %job_id, picture_id = %config.picture_id),
)]
pub async fn handle(
    client: &BackendClient,
    job_id: Uuid,
    claim_token: Uuid,
    config: EditPictureConfig,
    presigned_read: Option<String>,
    presigned_writes: PresignedWrites,
    mime_type: Option<String>,
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

    // ── Apply the EXIF edit (set/clear) into the file ─────────────────────────
    if let Some(ref edit) = config.exif {
        let path = file_path.clone();
        let set = edit.set.clone();
        let clear = edit.clear.clone();
        let mime_type = mime_type.clone();
        let span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            let _guard = span.enter();
            exif_mod::write_exif_overrides(&path, &set, &clear, mime_type.as_deref())
        })
        .await
        .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))??;
    }

    // ── Visual transforms ────────────────────────────────────────────────────
    // TODO: implement crop / resize once the imaging primitives are ready.
    if config.visual.is_some() {
        warn!(job_id = %job_id, "visual transforms not yet implemented; uploading original");
    }

    // ── Regenerate thumbnails (visual edits only) BEFORE the original upload ──
    // Keeping the original upload last preserves the file-untouched-on-failure invariant.
    let thumb = thumbnailer::run(client, &file_path, &presigned_writes, tmp.path()).await?;

    // ── File size + hash (after EXIF write, so values match what is uploaded) ─
    let file_size = std::fs::metadata(&file_path).map(|m| m.len() as i64).ok();

    let path_for_hash = file_path.clone();
    let file_hash =
        tokio::task::spawn_blocking(move || archypix_common::hash::hash_file(&path_for_hash))
            .await
            .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;
    if file_hash.is_none() {
        warn!(job_id = %job_id, "failed to compute file hash; skipping");
    }

    // Metadata-stripped content hash (feature 11): unchanged by an EXIF-only edit, refreshed by a
    // visual re-encode so the dedup reconciler regroups it. Enters the current (job) span like the
    // EXIF-write spawn_blocking task above.
    let path_for_content = file_path.clone();
    let content_span = tracing::Span::current();
    let content_hash = tokio::task::spawn_blocking(move || {
        let _guard = content_span.enter();
        content_hash_mod::content_hash(&path_for_content)
    })
    .await
    .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))?;

    // ── Upload modified original (last fallible step) ────────────────────────
    info!(job_id = %job_id, "edit_picture: uploading modified original");
    client.upload_presigned(&output_url, &file_path).await?;

    client
        .complete_job(
            job_id,
            CompleteJobRequest {
                claim_token,
                exif: None,
                blurhash: thumb.blurhash,
                thumbnails_generated: thumb.generated,
                file_size,
                file_hash,
                content_hash,
                width: thumb.width,
                height: thumb.height,
            },
        )
        .await?;

    info!(
        job_id = %job_id,
        thumbnails_regenerated = thumb.generated,
        "edit_picture completed"
    );
    Ok(())
}
