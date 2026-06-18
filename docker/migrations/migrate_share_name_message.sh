#!/usr/bin/env bash
# Migrate the test-env databases (archypix_back1/2/3) to add the share name/message columns
# without resetting their data. Adds the columns nullable, backfills existing rows, then sets
# `name` NOT NULL so the result matches the canonical schema (no lingering column default).
#
# Postgres runs in the `archypix-postgres` container; psql is invoked there via `docker exec`.
# Safe to re-run: every statement is idempotent.
set -euo pipefail

CONTAINER="${PG_CONTAINER:-archypix-postgres}"

for n in 1 2 3; do
  echo "==> migrating archypix_back${n}"
  docker exec -i "$CONTAINER" psql -U archypix -d "archypix_back${n}" -v ON_ERROR_STOP=1 <<'SQL'
    ALTER TABLE outgoing_shares ADD COLUMN IF NOT EXISTS name    VARCHAR(64);
    ALTER TABLE outgoing_shares ADD COLUMN IF NOT EXISTS message TEXT;
    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS name    VARCHAR(64);
    ALTER TABLE incoming_shares ADD COLUMN IF NOT EXISTS message TEXT;

    UPDATE outgoing_shares SET name = '' WHERE name IS NULL;
    UPDATE incoming_shares SET name = '' WHERE name IS NULL;

    ALTER TABLE outgoing_shares ALTER COLUMN name SET NOT NULL;
    ALTER TABLE incoming_shares ALTER COLUMN name SET NOT NULL;
SQL
done

echo "All test databases migrated."
