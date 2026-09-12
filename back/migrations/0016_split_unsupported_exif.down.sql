-- Fold the file verdicts back into the single unsupported bucket (0015's down then renames it).
UPDATE pictures SET exif_sync_status = 'unsupported_mime' WHERE exif_sync_status = 'unsupported_file';
