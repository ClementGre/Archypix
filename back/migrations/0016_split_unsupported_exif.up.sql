-- Feature 33 §9 follow-up (separate file so 0015's new labels are committed): promote the genuine
-- *file* verdicts out of the renamed MIME bucket. A row whose MIME **is** EXIF-capable was marked
-- unsupported by a worker that could not open the file, not by the preflight.
--
-- The allowlist is inlined as a one-time historical snapshot — the correct place for it, and the
-- last time it appears in SQL anywhere (§8: the verdict is stored, never derived).
UPDATE pictures
SET exif_sync_status = 'unsupported_file'
WHERE exif_sync_status = 'unsupported_mime'
  AND lower(mime_type) = ANY (ARRAY [
    'image/jpeg', 'image/jpg', 'image/png', 'image/tiff', 'image/tif', 'image/webp',
    'image/heic', 'image/heif', 'image/avif', 'image/bmp', 'image/x-bmp',
    'image/x-nikon-nef', 'image/x-canon-cr2', 'image/x-canon-cr3', 'image/x-sony-arw',
    'image/x-fuji-raf', 'image/x-adobe-dng', 'image/x-panasonic-rw2'
    ]::text[]);
