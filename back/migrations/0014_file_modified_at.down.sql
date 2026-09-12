DROP TRIGGER IF EXISTS update_pictures_file_modified_at ON pictures;
DROP FUNCTION IF EXISTS update_file_modified_at_column();
ALTER TABLE pictures
    DROP COLUMN IF EXISTS file_modified_at;
