-- Drop the (0,0) "no fix" sentinel some writers stamp (feature 30 §12.11). The promoted columns
-- and the file_exif snapshot are cleared together so feature 31's reconciler sees no drift and
-- never writes back to the file.
UPDATE pictures
SET gps_lat = NULL,
    gps_lng = NULL,
    gps_alt = NULL
WHERE gps_lat = 0 AND gps_lng = 0;

UPDATE pictures
SET file_exif = file_exif - 'gps_lat' - 'gps_lng' - 'gps_alt'
WHERE (file_exif ->> 'gps_lat')::float8 = 0
  AND (file_exif ->> 'gps_lng')::float8 = 0;
