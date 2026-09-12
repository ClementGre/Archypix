-- Feature 32: a WebDAV last-modified that moves only when the bytes move.
-- `updated_at` is bumped by any row write (tagging included), which makes mtime-comparing sync
-- clients re-download re-tagged pictures. See doc/features/32_webdav_file_modified_at.md.

ALTER TABLE pictures
    ADD COLUMN file_modified_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'utc');

-- Backfill from `updated_at`: for a picture not re-tagged since a client's last sync this is
-- exactly the value that client already recorded, so the migration causes no extra churn.
UPDATE pictures
SET file_modified_at = updated_at;

-- Stamped from `file_hash` transitions (the ETag), so mtime and ETag agree by construction.
CREATE OR REPLACE FUNCTION update_file_modified_at_column()
    RETURNS TRIGGER AS
$$
BEGIN
    NEW.file_modified_at = (now() at time zone 'utc');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_pictures_file_modified_at
    BEFORE UPDATE
    ON pictures
    FOR EACH ROW
    WHEN (NEW.file_hash IS DISTINCT FROM OLD.file_hash)
EXECUTE FUNCTION update_file_modified_at_column();
