#!/usr/bin/env bash
# Apply pending migrations to every dev/test database (the dev DB plus the federation test stack).
# Run from anywhere; `cargo sqlx migrate` is invoked from `back/` so it finds `migrations/`.
# Override the set with e.g. `DBS="archypix_back" ./scripts/migrate.sh`.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_URL="${PG_BASE_URL:-postgres://archypix:archypix@localhost:5432}"
DBS="${DBS:-archypix_back archypix_back1 archypix_back2 archypix_back3}"

cd "${SCRIPT_DIR}/.."
for db in $DBS; do
  echo "==> migrating ${db}"
  DATABASE_URL="${BASE_URL}/${db}" cargo sqlx migrate run
done

echo "Migrations applied. Remember: cargo sqlx prepare -- --tests, and regenerate migrations/schema.sql."
