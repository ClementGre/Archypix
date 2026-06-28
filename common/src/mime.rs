/// MIME type support whitelists for worker image processing.
///
/// Workers check the picture's `mime_type` field before attempting EXIF extraction
/// or thumbnail generation to avoid feeding unsupported formats to native libraries.

/// MIME types that GExiv2 (rexiv2) can read and write EXIF metadata for.
pub const MIME_TYPES_EXIF: &[&str] = &[
    "image/jpeg",
    "image/jpg", // non-standard alias, common in the wild
    "image/png",
    "image/tiff",
    "image/tif", // non-standard alias
    "image/webp",
    "image/heic",
    "image/heif",
    "image/avif",
    "image/bmp",
    "image/x-bmp",
    // Common camera raw formats (GExiv2 reads EXIF from many raw formats)
    "image/x-nikon-nef",
    "image/x-canon-cr2",
    "image/x-canon-cr3",
    "image/x-sony-arw",
    "image/x-fuji-raf",
    "image/x-adobe-dng",
    "image/x-panasonic-rw2",
];

/// Image MIME types that ImageMagick can decode and from which WebP thumbnails can be generated.
/// This is the *image engine* whitelist — videos are thumbnailed separately (see `MIME_TYPES_VIDEO`
/// / `supports_thumbnail`).
pub const MIME_TYPES_IMAGE_THUMBNAIL: &[&str] = &[
    "image/jpeg",
    "image/jpg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/tif",
    "image/bmp",
    "image/x-bmp",
    "image/heic",
    "image/heif",
    "image/avif",
    "image/ico",
    "image/x-icon",
    "image/pnm",
    "image/x-portable-anymap",
];

/// Video MIME types handled via ffmpeg/ffprobe: container metadata extraction (capture date, GPS,
/// rotation, make/model, duration, codecs) and a frame-grab feeding the WebP thumbnail pipeline.
/// Deliberately disjoint from `MIME_TYPES_EXIF`/`MIME_TYPES_IMAGE_THUMBNAIL` — videos take a separate
/// worker path (ffmpeg, not GExiv2/ImageMagick) and a DB-only EXIF edit (no embedded-metadata write).
pub const MIME_TYPES_VIDEO: &[&str] = &[
    "video/mp4",
    "video/quicktime", // .mov
    "video/webm",
    "video/x-matroska", // .mkv
    "video/x-msvideo",  // .avi
    "video/mpeg",
    "video/3gpp",
    "video/x-m4v",
    "video/ogg",
];

/// Returns `true` when GExiv2 supports EXIF extraction for this MIME type.
pub fn supports_exif(mime_type: &str) -> bool {
    let lower = mime_type.to_lowercase();
    MIME_TYPES_EXIF.contains(&lower.as_str())
}

/// Returns `true` when ImageMagick can decode this image MIME type — i.e. the image-engine path.
/// For "will this picture have a thumbnail at all?" use [`supports_thumbnail`], which also covers
/// videos (frame-grab).
pub fn supports_image_thumbnail(mime_type: &str) -> bool {
    let lower = mime_type.to_lowercase();
    MIME_TYPES_IMAGE_THUMBNAIL.contains(&lower.as_str())
}

/// Returns `true` when this is a video MIME type handled via the ffmpeg path.
pub fn supports_video(mime_type: &str) -> bool {
    let lower = mime_type.to_lowercase();
    MIME_TYPES_VIDEO.contains(&lower.as_str())
}

/// Every MIME type a thumbnail can be generated for — the image-engine list (ImageMagick) plus the
/// video-engine list (ffmpeg frame-grab). Single source of truth behind both [`supports_thumbnail`]
/// and callers that need the explicit set (e.g. the `mime_type = ANY(...)` filter in the
/// thumbnail-regen sweep).
pub fn thumbnailable_mimes() -> impl Iterator<Item = &'static str> {
    MIME_TYPES_IMAGE_THUMBNAIL
        .iter()
        .chain(MIME_TYPES_VIDEO)
        .copied()
}

/// Returns `true` when a thumbnail can be generated for this format **at all** — an image (decoded
/// by ImageMagick) or a video (an ffmpeg frame-grab). This is the predicate callers should use to
/// decide whether a picture is expected to gain small/medium/large thumbnails; the worker picks the
/// concrete engine with [`supports_image_thumbnail`] / [`supports_video`].
pub fn supports_thumbnail(mime_type: &str) -> bool {
    supports_image_thumbnail(mime_type) || supports_video(mime_type)
}
