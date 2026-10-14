-- A sensitive tag veils every picture at or under it in the web viewer (feature 38).
ALTER TABLE tag_metadata ADD COLUMN sensitive boolean NOT NULL DEFAULT false;

CREATE TYPE sensitive_display_mode AS ENUM ('visible', 'blurred', 'redacted', 'hidden');
ALTER TABLE user_settings
    ADD COLUMN sensitive_display_mode sensitive_display_mode NOT NULL DEFAULT 'blurred';
