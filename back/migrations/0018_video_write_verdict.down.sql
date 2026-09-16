-- Restore the pre-amendment rule: a video that was read successfully sits in `synced`.
--
-- `file_exif IS NOT NULL` is the successful-read witness (§4.7) and so selects exactly the set the
-- up migration moved — plus the few rows an edit had already stamped `unsupported_mime` under the
-- old lazy path, which the two states do not distinguish. Harmless: the old rule would have put
-- those back on `synced` at their next re-extraction anyway.
UPDATE pictures
SET exif_sync_status = 'synced'
WHERE exif_sync_status = 'unsupported_mime'
  AND remote_picture_id IS NULL
  AND file_exif IS NOT NULL
  AND lower(mime_type) = ANY (ARRAY [
    'video/mp4', 'video/quicktime', 'video/webm', 'video/x-matroska', 'video/x-msvideo',
    'video/mpeg', 'video/3gpp', 'video/x-m4v', 'video/ogg'
    ]::text[]);
