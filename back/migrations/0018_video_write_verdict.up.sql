-- Feature 33 §4.6 amended: a successful read no longer settles a video on `synced`.
--
-- A video is readable (ffprobe) and never writable, so its write verdict is known the moment the
-- extraction lands. Leaving it `synced` made the stored state incomplete, and the set-based batch
-- path — which §10 has partition on status alone — enqueued a doomed write job, landing the row on
-- `unsupported_file` ("unreadable") instead of `unsupported_mime` ("n/a").
--
-- Backfill the rows already ingested under the old rule. Owned only: for a received row the column
-- is inert (§4.7) and `synced` is the value that keeps it out of every write path.
--
-- The video allowlist is inlined as a one-time historical snapshot, as in 0016.
UPDATE pictures
SET exif_sync_status = 'unsupported_mime'
WHERE exif_sync_status = 'synced'
  AND remote_picture_id IS NULL
  AND lower(mime_type) = ANY (ARRAY [
    'video/mp4', 'video/quicktime', 'video/webm', 'video/x-matroska', 'video/x-msvideo',
    'video/mpeg', 'video/3gpp', 'video/x-m4v', 'video/ogg'
    ]::text[]);
