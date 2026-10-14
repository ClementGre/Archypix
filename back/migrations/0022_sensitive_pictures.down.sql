ALTER TABLE user_settings DROP COLUMN IF EXISTS sensitive_display_mode;
DROP TYPE IF EXISTS sensitive_display_mode;
ALTER TABLE tag_metadata DROP COLUMN IF EXISTS sensitive;
