use crate::error::{Result, WorkerError};
use magick_rust::{MagickWand, magick_wand_genesis};
use std::path::Path;
use std::sync::Once;
use tracing::debug;

static MAGICK_INIT: Once = Once::new();

/// Initialize ImageMagick once per process. Safe to call multiple times.
fn init_magick() {
    MAGICK_INIT.call_once(|| {
        magick_wand_genesis();
    });
}

/// Named thumbnail variants and their target heights in pixels.
pub const THUMBNAIL_VARIANTS: &[(&str, usize)] =
    &[("small", 100), ("medium", 500), ("large", 1000)];

/// Read the raw decoded pixel dimensions `(width, height)` of the image at `src`.
///
/// These are the stored-raster dimensions (EXIF orientation applied at display time), so they match
/// the raw-pixel thumbnails/blurhash. Authoritative source of `pictures.width/height`. Must run
/// inside `tokio::task::spawn_blocking`.
pub fn image_dimensions(src: &Path) -> Result<(i32, i32)> {
    init_magick();

    let wand = MagickWand::new();
    wand.read_image(src.to_str().unwrap())
        .map_err(|e| WorkerError::Imaging(format!("read image: {e}")))?;

    let w = wand.get_image_width();
    let h = wand.get_image_height();
    if w == 0 || h == 0 {
        return Err(WorkerError::Imaging(
            "image has zero dimensions".to_string(),
        ));
    }
    Ok((w as i32, h as i32))
}

/// Generate a WebP thumbnail at the specified height (maintaining aspect ratio).
///
/// Writes the result to `dest_path`. Must run inside `tokio::task::spawn_blocking`.
pub fn generate_thumbnail(src: &Path, dest: &Path, target_height: usize) -> Result<()> {
    init_magick();

    let mut wand = MagickWand::new();
    wand.read_image(src.to_str().unwrap())
        .map_err(|e| WorkerError::Imaging(format!("read image: {e}")))?;

    let orig_w = wand.get_image_width();
    let orig_h = wand.get_image_height();
    if orig_h == 0 {
        return Err(WorkerError::Imaging("image has zero height".to_string()));
    }

    let target_width = (target_height * orig_w) / orig_h;
    wand.thumbnail_image(target_width, target_height)
        .map_err(|e| WorkerError::Imaging(format!("thumbnail: {e}")))?;

    wand.set_image_format("webp")
        .map_err(|e| WorkerError::Imaging(format!("set format: {e}")))?;

    wand.write_image(dest.to_str().unwrap())
        .map_err(|e| WorkerError::Imaging(format!("write: {e}")))?;

    debug!(
        src = %src.display(),
        dest = %dest.display(),
        target_height,
        "thumbnail generated"
    );
    Ok(())
}

/// Generate a BlurHash string for an image.
///
/// Must run inside `tokio::task::spawn_blocking`.
pub fn generate_blurhash(src: &Path) -> Result<String> {
    init_magick();

    let wand = MagickWand::new();
    wand.read_image(src.to_str().unwrap())
        .map_err(|e| WorkerError::Imaging(format!("read image for blurhash: {e}")))?;

    let w = wand.get_image_width();
    let h = wand.get_image_height();
    if w == 0 || h == 0 {
        return Err(WorkerError::Imaging(
            "image has zero dimensions".to_string(),
        ));
    }

    // Choose component counts based on orientation.
    let (cx, cy) = if w > h {
        (4, 3)
    } else if w == h {
        (3, 3)
    } else {
        (3, 4)
    };

    let raw = wand
        .export_image_pixels(0, 0, w, h, "RGBA")
        .ok_or_else(|| WorkerError::Imaging("failed to export pixels".to_string()))?;

    blurhash::encode(cx as u32, cy as u32, w as u32, h as u32, &raw)
        .map_err(|e| WorkerError::Imaging(format!("blurhash encode: {e:?}")))
}
