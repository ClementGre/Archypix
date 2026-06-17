# Coding Guidelines

## Database migrations

**There is one migration file: `001_initial_schema.up.sql`. All schema changes are made directly in that file. Do not add more migration files.**

When editing the database schema, migrate the database with `cd back && cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare`

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
