#!/usr/bin/env bash
# Add trace_context JSONB column to the jobs table for observability feature (12).
# Safe to re-run: ALTER TABLE ... ADD COLUMN IF NOT EXISTS is idempotent.
set -euo pipefail

CONTAINER="${PG_CONTAINER:-archypix-postgres}"

for n in 1 2 3; do
  echo "==> migrating archypix_back${n}"
  docker exec -i "$CONTAINER" psql -U archypix -d "archypix_back${n}" -v ON_ERROR_STOP=1 <<'SQL'
    ALTER TABLE jobs ADD COLUMN IF NOT EXISTS trace_context JSONB;
SQL
done

echo "All test databases migrated."
