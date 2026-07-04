DROP TRIGGER IF EXISTS trg_user_storage_versions ON picture_versions;
DROP TRIGGER IF EXISTS trg_user_storage_pictures_del ON pictures;
DROP TRIGGER IF EXISTS trg_user_storage_pictures_iu ON pictures;
DROP FUNCTION IF EXISTS user_storage_versions();
DROP FUNCTION IF EXISTS user_storage_pictures();
DROP TABLE IF EXISTS user_storage;
ALTER TABLE users
    DROP COLUMN IF EXISTS storage_quota_bytes;
