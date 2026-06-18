#!/usr/bin/env bash
# Editing `back/migrations/001_initial_schema.up.sql` in place changes its sqlx checksum, so any
# already-migrated database (the test stack: archypix_back1/2/3) refuses to start with:
#   "migration 1 was previously applied but has been modified".
#
# sqlx stores the SHA-384 digest of the up.sql bytes in `_sqlx_migrations.checksum`. This script
# recomputes that digest from the current schema file and writes it back for version 1, so the
# seeded test databases accept the modified migration without being reset.
#
# Run it after every schema edit (alongside the data-migration script for that change).
# Postgres runs in the `archypix-postgres` container; psql is invoked there via `docker exec`.
# Safe to re-run.
set -euo pipefail

CONTAINER="${PG_CONTAINER:-archypix-postgres}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UP_SQL="${SCRIPT_DIR}/../../back/migrations/001_initial_schema.up.sql"

CHECKSUM="$(shasum -a 384 "$UP_SQL" | awk '{print $1}')"
echo "current up.sql SHA-384: ${CHECKSUM}"

# The dev DB (archypix_back) is rebuilt via `sqlx migrate revert/run`; only the seeded test DBs
# need patching. Override with e.g. `DBS="archypix_back archypix_back1"` if needed.
DBS="${DBS:-archypix_back1 archypix_back2 archypix_back3}"

for db in $DBS; do
  echo "==> patching ${db} _sqlx_migrations version 1"
  docker exec -i "$CONTAINER" psql -U archypix -d "$db" -v ON_ERROR_STOP=1 \
    -c "UPDATE _sqlx_migrations SET checksum = decode('${CHECKSUM}', 'hex') WHERE version = 1;"
done

echo "Migration checksums updated."
