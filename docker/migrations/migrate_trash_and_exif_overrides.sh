#!/usr/bin/env bash
# Migrate the test-env databases (archypix_back1/2/3) for feature 09 (Trash, owner-deletion
# propagation & recipient EXIF overrides) and the consolidated 10/11 columns, without resetting
# their data:
#   new enum picture_deleted_reason
#   pictures.owner_deleted_at / owner_purge_at / remote_exif_data / local_exif_overrides /
#           deleted_reason / content_hash / copy_source_owner_username / copy_source_owner_instance /
#           copy_source_picture_id
#   user_settings.trash_retention_days
#   outgoing_shares.allow_exif_edit / incoming_shares.allow_exif_edit
#   indexes idx_pictures_owned_trashed / idx_pictures_content_hash
#
# All columns are nullable (or carry the canonical DEFAULT), so existing rows backfill with no extra
# step. Postgres runs in the `archypix-postgres` container; psql is invoked via `docker exec`.
# Safe to re-run: every statement is idempotent.
set -euo pipefail

CONTAINER="${PG_CONTAINER:-archypix-postgres}"

for n in 1 2 3; do
  echo "==> migrating archypix_back${n}"
  docker exec -i "$CONTAINER" psql -U archypix -d "archypix_back${n}" -v ON_ERROR_STOP=1 <<'SQL'
    DO $$ BEGIN
      CREATE TYPE picture_deleted_reason AS ENUM ('manual', 'boomerang', 'content_dedupe');
    EXCEPTION WHEN duplicate_object THEN NULL; END $$;

    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS owner_deleted_at TIMESTAMP;
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS owner_purge_at TIMESTAMP;
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS remote_exif_data JSONB;
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS local_exif_overrides JSONB;
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS deleted_reason picture_deleted_reason;
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS content_hash TEXT;
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS copy_source_owner_username VARCHAR(255);
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS copy_source_owner_instance VARCHAR(255);
    ALTER TABLE pictures ADD COLUMN IF NOT EXISTS copy_source_picture_id VARCHAR(255);

    CREATE INDEX IF NOT EXISTS idx_pictures_owned_trashed ON pictures (deleted_at)
        WHERE deleted_at IS NOT NULL AND remote_picture_id IS NULL;
    CREATE INDEX IF NOT EXISTS idx_pictures_content_hash ON pictures (local_user_id, content_hash)
        WHERE content_hash IS NOT NULL;

    ALTER TABLE user_settings ADD COLUMN IF NOT EXISTS trash_retention_days INT NOT NULL DEFAULT 30;

    ALTER TABLE outgoing_shares ADD COLUMN IF NOT EXISTS allow_exif_edit BOOLEAN NOT NULL DEFAULT FALSE;
    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS allow_exif_edit BOOLEAN NOT NULL DEFAULT FALSE;
SQL
done

echo "All test databases migrated."
