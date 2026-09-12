-- Feature 33 §9: the EXIF read path becomes observable.

-- `unsupported` was overwhelmingly the MIME-preflight verdict, so it is renamed rather than
-- migrated. Safe only because no old backend binary runs against the migrated DB (§12 C): the Rust
-- enum maps labels by name.
ALTER TYPE public.picture_exif_sync_status RENAME VALUE 'unsupported' TO 'unsupported_mime';

-- The read-direction states. `ADD VALUE` only *adds* the label (never *used* in this migration),
-- so it is safe inside the migration transaction (the 31 §2.1 pattern).
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'unsupported_file';
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'extracting';
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'extract_failed';

-- Every value is now an observation: each insert path states its own status (§4.1). A default
-- would assume an extraction follows, which three of the four insert paths do not do.
ALTER TABLE pictures ALTER COLUMN exif_sync_status DROP DEFAULT;
