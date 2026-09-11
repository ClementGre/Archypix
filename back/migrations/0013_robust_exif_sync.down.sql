-- `write_failed` rows become `pending` again before the column disappears; Postgres cannot drop a
-- value from an enum type, so the label itself is left in place (as in 0003).
UPDATE pictures SET exif_sync_status = 'pending' WHERE exif_sync_status = 'write_failed';

ALTER TABLE pictures DROP COLUMN IF EXISTS file_exif;
