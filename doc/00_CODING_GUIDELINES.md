# Coding Guidelines

## Database migrations

All schema changes go directly into the single file `back/migrations/001_initial_schema.up.sql` (+ its
`.down.sql`); never add more migration files. After editing it:

1. **Rebuild the dev DB** (from `back/`, `DATABASE_URL=…/archypix_back`):
   `cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare -- --tests`
   (`-- --tests` captures test query macros in `.sqlx`; verify with
   `env -u DATABASE_URL SQLX_OFFLINE=true cargo check --tests -p archypix-back`).
2. **Migrate the seeded test DBs** `archypix_back1/2/3` (don't reset them): write/extend an idempotent
   script in `docker/migrations/` (add columns nullable → backfill → `SET NOT NULL`, using
   `ADD COLUMN IF NOT EXISTS` etc. so it re-runs safely).
3. **Fix the sqlx checksum** on those test DBs, else they refuse to start (*"migration 1 … has been
   modified"*): run `docker/migrations/fix_migration_checksum.sh` (recomputes the `up.sql` SHA-384 into
   `_sqlx_migrations.checksum`).

Postgres runs in the `archypix-postgres` container (`archypix` superuser, port 5432); `psql` is not on
the host, so use `docker exec -i archypix-postgres psql -U archypix -d archypix_back{n} …`. The four DBs
(`archypix_back`, `archypix_back1/2/3`) are created by `docker/postgres-init.sql`. See the two scripts in
`docker/migrations/` as working examples.

## Rust guidelines

Follow Rust best practices. Always favor refactoring over sticking to existing legacy functions.

For modules with sub-files, use a `module_name.rs` file alongside the `module_name/` directory instead of placing a `mod.rs` inside the directory.

# Comon mistakes

- Global domain comparaison can’t tell if the instances are the same. bob_global_domain == alice_global_domain does not tell if bob and alice are on
  the same instance. Multiple instances can have the same global domain. Use `services::users::find_local_user_id` instead to check if a user is on
  the same instance.

# Environment

For things involving the archypix-worker crate, run in `nix develop`.

# Agents

When making changes to the codebase:

- Keep documentation up to date. Match the level of detail already present — do not add overly specific descriptions of what was changed beyond what
  the rest of the doc covers.
- When editing the api, update `06_API_REFERENCE.md`.
- When completing a task, update `99_ROADMAP_MVP.md`, and eventually add things not implemented yet into it.
- On back, keep tests up to date. New features and modified behaviour should be reflected in the test suite.
- On front, don’t start or preview the frontend server yourself. Only check that it builds. The user can give you feedback on the frontend
  changes.
