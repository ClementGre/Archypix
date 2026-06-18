#!/usr/bin/env bash
# Migrate the test-env databases (archypix_back1/2/3) to add the new share-tracking columns without
# resetting their data:
#   outgoing_shares.shareback_of
#   incoming_shares.future / shared_tag_path / last_announcement_received_at / shareback_of
#
# All columns are nullable (or carry the canonical DEFAULT FALSE), so existing rows backfill with no
# extra step. Postgres runs in the `archypix-postgres` container; psql is invoked via `docker exec`.
# Safe to re-run: every statement is idempotent.
set -euo pipefail

CONTAINER="${PG_CONTAINER:-archypix-postgres}"

for n in 1 2 3; do
  echo "==> migrating archypix_back${n}"
  docker exec -i "$CONTAINER" psql -U archypix -d "archypix_back${n}" -v ON_ERROR_STOP=1 <<'SQL'
    ALTER TABLE outgoing_shares ADD COLUMN IF NOT EXISTS shareback_of UUID;

    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS future BOOLEAN NOT NULL DEFAULT FALSE;
    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS shared_tag_path LTREE;
    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS last_announcement_received_at TIMESTAMP;
    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS shareback_of UUID;
SQL
done

echo "All test databases migrated."
