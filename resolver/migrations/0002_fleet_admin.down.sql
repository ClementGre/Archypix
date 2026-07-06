DROP TABLE IF EXISTS resolver_admin;
DROP TABLE IF EXISTS invites;
DROP TABLE IF EXISTS resolver_settings;
ALTER TABLE backends
    DROP COLUMN IF EXISTS delegation_token,
    DROP COLUMN IF EXISTS delegation_expires_at,
    DROP COLUMN IF EXISTS user_count,
    DROP COLUMN IF EXISTS picture_count,
    DROP COLUMN IF EXISTS storage_bytes,
    DROP COLUMN IF EXISTS last_heartbeat_at,
    DROP COLUMN IF EXISTS healthy,
    DROP COLUMN IF EXISTS reachable,
    DROP COLUMN IF EXISTS accepting_registrations,
    DROP COLUMN IF EXISTS max_users,
    DROP COLUMN IF EXISTS version,
    DROP COLUMN IF EXISTS last_selected_at;
