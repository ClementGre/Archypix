-- Horizontal position error radius in metres, mirroring EXIF GPSHPositioningError (feature 36).
-- NULL = unstated. Existing rows are populated by the admin `synced` re-extract sweep (36 §5).
ALTER TABLE pictures ADD COLUMN IF NOT EXISTS gps_accuracy_m double precision;
