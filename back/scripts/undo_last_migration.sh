#!/usr/bin/env bash
# Revert the most recent migration on every dev/test database (see migrate.sh for the DB set).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_URL="${PG_BASE_URL:-postgres://archypix:archypix@localhost:5432}"
DBS="${DBS:-archypix_back archypix_back1 archypix_back2 archypix_back3}"

cd "${SCRIPT_DIR}/.."
for db in $DBS; do
  echo "==> reverting last migration on ${db}"
  DATABASE_URL="${BASE_URL}/${db}" cargo sqlx migrate revert
done

echo "Last migration reverted."
