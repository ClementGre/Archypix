-- Postgres cannot drop an enum value (see the 0013 down), so the read-direction labels are folded
-- back and left orphaned (§9, §12 G).
UPDATE pictures SET exif_sync_status = 'synced'
 WHERE exif_sync_status IN ('extracting', 'extract_failed');
UPDATE pictures SET exif_sync_status = 'unsupported_mime' WHERE exif_sync_status = 'unsupported_file';

ALTER TYPE public.picture_exif_sync_status RENAME VALUE 'unsupported_mime' TO 'unsupported';
ALTER TABLE pictures ALTER COLUMN exif_sync_status SET DEFAULT 'synced';
