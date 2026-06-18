//! Factorized thumbnail generation + upload step.
//!
//! Used by both `gen_thumbnail` and `edit_picture` handlers so the logic lives
//! in exactly one place.  The caller owns the `TempDir` (for RAII cleanup) and
//! passes the directory path in.

use crate::backend::BackendClient;
use crate::error::{Result, WorkerError};
use crate::imaging::resize;
use archypix_common::transfer::PresignedWrites;
use std::path::Path;
use tracing::{debug, warn};

pub struct ThumbnailOutput {
    /// BlurHash string, if generation succeeded (failure is non-fatal).
    pub blurhash: Option<String>,
    /// `true` when all three variants were generated and uploaded.
    pub generated: bool,
    /// Raw decoded pixel dimensions of the source image (authoritative for `pictures.width/height`).
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Generate WebP thumbnails and a BlurHash from the image at `src`, then
/// upload each thumbnail to the corresponding presigned PUT URL in `writes`.
///
/// Returns immediately with `generated = false` when `writes` contains no
/// thumbnail slots (EXIF-only edits, ML jobs, etc.).
///
/// The caller must pass a writable `tmp_dir` for the intermediate WebP files.
/// The directory is NOT cleaned up here — the caller is responsible for that
/// (typically by keeping a `TempDir` alive for the duration of the job).
pub async fn run(
    client: &BackendClient,
    src: &Path,
    writes: &PresignedWrites,
    tmp_dir: &Path,
) -> Result<ThumbnailOutput> {
    if !writes.has_thumbnails() {
        return Ok(ThumbnailOutput {
            blurhash: None,
            generated: false,
            width: None,
            height: None,
        });
    }

    let src_c = src.to_path_buf();
    let dir_c = tmp_dir.to_path_buf();

    // All image work is CPU-bound — must run on a blocking thread.
    let (paths, blurhash, dimensions) = tokio::task::spawn_blocking(move || -> Result<_> {
        // Decoded dimensions first — the authoritative source for pictures.width/height.
        let dimensions = resize::image_dimensions(&src_c)?;

        let mut paths: Vec<(String, std::path::PathBuf)> = Vec::new();
        for &(name, height) in resize::THUMBNAIL_VARIANTS {
            let dest = dir_c.join(format!("{name}.webp"));
            resize::generate_thumbnail(&src_c, &dest, height)
                .map_err(|e| WorkerError::Imaging(format!("'{name}' thumbnail: {e}")))?;
            paths.push((name.to_string(), dest));
        }

        let blurhash = match resize::generate_blurhash(&src_c) {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(error = ?e, "blurhash generation failed; skipping");
                None
            }
        };

        Ok((paths, blurhash, dimensions))
    })
    .await
    .map_err(|e| WorkerError::Imaging(format!("spawn_blocking panicked: {e}")))??;

    // Upload each generated thumbnail to its presigned URL.
    // thumbnail_pairs() only yields slots that are Some, and has_thumbnails()
    // above guarantees all three are populated, so no path will be skipped.
    for (variant, url) in writes.thumbnail_pairs() {
        if let Some((_, path)) = paths.iter().find(|(n, _)| n == variant) {
            debug!(variant, "uploading thumbnail");
            client.upload_presigned(url, path).await?;
        }
    }

    Ok(ThumbnailOutput {
        blurhash,
        generated: true,
        width: Some(dimensions.0),
        height: Some(dimensions.1),
    })
}
