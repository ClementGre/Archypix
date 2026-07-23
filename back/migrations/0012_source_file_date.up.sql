-- Feature 30 §10: source file date (suggestion-only, never auto-applied to captured_at).
-- Populated at ingest from the browser's File.lastModified (upload) or an X-OC-Mtime header
-- (WebDAV PUT); NULL otherwise. Surfaced on the list/detail so the date-fix chip can offer it.
ALTER TABLE pictures
    ADD COLUMN original_file_created_at timestamp without time zone;
