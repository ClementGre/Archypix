-- Feature 15: new gallery sort options (file size, filename). Index them per-user so sorting the
-- flat list ("all photos" / scope views) is efficient, matching the user-scoped list query
-- (`WHERE local_user_id = $1 ORDER BY <col>`).

CREATE INDEX idx_pictures_user_file_size ON pictures (local_user_id, file_size);
CREATE INDEX idx_pictures_user_filename ON pictures (local_user_id, filename);
