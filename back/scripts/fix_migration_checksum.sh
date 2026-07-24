#!/usr/bin/env bash
# Editing an already-applied migration in `back/migrations/*.up.sql` changes its sqlx checksum, so
# any already-migrated database (the test stack: archypix_back1/2/3) refuses to start with:
#   "migration <n> was previously applied but has been modified".
#
# sqlx stores the SHA-384 digest of each up.sql's bytes in `_sqlx_migrations.checksum`, keyed by the
# migration's version (the numeric filename prefix, e.g. `0012_...` -> version 12). This script
# recomputes the digest for every migration file and, for each DB, patches the stored checksum of any
# version that exists both on disk and in `_sqlx_migrations` whose digest no longer matches. Versions
# absent from a DB, or whose checksum already matches, are left untouched.
#
# Run it after every schema edit (alongside the data-migration script for that change).
# Postgres runs in the `archypix-postgres` container; psql is invoked there via `docker exec`.
# Safe to re-run.
set -euo pipefail

CONTAINER="${PG_CONTAINER:-archypix-postgres}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MIGRATIONS_DIR="${SCRIPT_DIR}/../migrations"

# Build one (version, checksum) VALUES row per migration file.
VALUES_SQL=""
for f in "$MIGRATIONS_DIR"/*.up.sql; do
  base="$(basename "$f")"
  version="$((10#${base%%_*}))"   # 10# forces base-10 so leading zeros aren't read as octal
  checksum="$(shasum -a 384 "$f" | awk '{print $1}')"
  [ -n "$VALUES_SQL" ] && VALUES_SQL+=$',\n'
  VALUES_SQL+="  (${version}, decode('${checksum}', 'hex'))"
done

# Update only the versions present in the DB whose stored checksum differs; RETURNING lists what
# actually changed (empty result => nothing to patch).
SQL="UPDATE _sqlx_migrations m
SET checksum = v.checksum
FROM (VALUES
${VALUES_SQL}
) AS v(version, checksum)
WHERE m.version = v.version
  AND m.checksum IS DISTINCT FROM v.checksum
RETURNING m.version;"

# The dev DB (archypix_back) is rebuilt via `sqlx migrate revert/run`; only the seeded test DBs
# need patching. Override with e.g. `DBS="archypix_back archypix_back1"` if needed.
DBS="${DBS:-archypix_back archypix_back1 archypix_back2 archypix_back3}"

for db in $DBS; do
  echo "==> patching ${db} _sqlx_migrations"
  printf '%s\n' "$SQL" | docker exec -i "$CONTAINER" psql -U archypix -d "$db" -v ON_ERROR_STOP=1 -f -
done

echo "Migration checksums updated."
