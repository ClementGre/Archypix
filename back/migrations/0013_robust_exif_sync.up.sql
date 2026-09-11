-- Feature 31: state-based EXIF synchronisation.

-- The authoritative snapshot of the EXIF embedded in the S3 original. NULL for received pictures
-- and for rows whose file has never been extracted.
ALTER TABLE pictures ADD COLUMN IF NOT EXISTS file_exif JSONB;

-- Backfill the rows already known to be in sync, so "revert to file" works without waiting for a
-- re-extraction. Shape matches `FullExif` (promoted fields + flattened camera keys).
UPDATE pictures
SET file_exif = jsonb_strip_nulls(jsonb_build_object(
        'captured_at', captured_at,
        'gps_lat', gps_lat,
        'gps_lng', gps_lng,
        'gps_alt', gps_alt,
        'orientation', orientation
                                  )) || COALESCE(exif_data, '{}'::jsonb)
WHERE file_exif IS NULL
  AND remote_picture_id IS NULL
  AND thumbnails_generated_at IS NOT NULL
  AND exif_sync_status = 'synced';

-- A permanent file-write failure: the DB holds the user's edit, the file still holds `file_exif`.
-- `ADD VALUE` only *adds* the label (it is never *used* in this migration), so it is safe inside
-- the migration transaction.
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'write_failed';
