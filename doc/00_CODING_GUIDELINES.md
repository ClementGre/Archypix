# Coding Guidelines

## Database migrations

Schema changes go into new migration files by default; only edit an already-applied migration if explicitly asked or if not yet applied in production.

```bash
cargo sqlx migrate add -r --sequential <name>   # creates xxx_<name>.up.sql / .down.sql
```

`back/migrations/schema.sql` is a generated, non-authoritative snapshot of the **full current schema**
— read this file (not the individual migration files) when you need to see the schema as it stands
today. Regenerate it after every migration:

```bash
docker exec -i archypix-postgres pg_dump -U archypix -d archypix_back --schema-only --no-owner \
  --no-privileges --no-comments --schema=public --exclude-table=_sqlx_migrations \
  | grep -vE '^--|^SET |^SELECT pg_catalog\.set_config|^\\restrict|^\\unrestrict' | cat -s \
  > back/migrations/schema.sql
```

After adding a migration:

1. **Apply it to the dev DB and regenerate the offline cache** (from `back/`, `DATABASE_URL=…/archypix_back`):
   `cargo sqlx migrate run && cargo sqlx prepare -- --tests`
   (`-- --tests` captures test query macros in `.sqlx`; verify with
   `env -u DATABASE_URL SQLX_OFFLINE=true cargo check --tests -p archypix-back`).
3. **Regenerate `schema.sql`** (command above) so it reflects the new schema.


## Rust guidelines

Follow Rust best practices. Always favor refactoring over sticking to existing legacy functions.

For modules with sub-files, use a `module_name.rs` file alongside the `module_name/` directory instead of placing a `mod.rs` inside the directory.

## Tracing

Use `#[tracing::instrument]` with `fields(...)` for identifying context instead of logging it at
the call site: don't repeat a field already on the span (own or ancestor's); log calls should
just carry genuinely new info (errors, counts, computed values). Use empty fields +
`Span::current().record(...)` for values only known partway through the function.

In `fields(...)`, a bare `name` declares an empty field — it does **not** capture the in-scope
variable (unlike `span!`/`event!`). Use `name = value` (or `%name`/`?name` shorthand) to actually
record it.

`AppError`-based error responses are already logged by `AppError::into_response()` — no need for
an extra `warn!` next to a function that just returns `AppError`.

Federation calls propagate trace context via headers (`trace_headers_for`/
`maybe_set_remote_parent` in `back/src/infra/observability.rs`, gated on the JWT-verified peer);
worker jobs propagate it through the DB job row instead.

# Common mistakes

- Global domain comparaison can’t tell if the instances are the same. bob_global_domain == alice_global_domain does not tell if bob and alice are on
  the same instance. Multiple instances can have the same global domain. Use `services::users::find_local_user_id` instead to check if a user is on
  the same instance.

# Environment

For things involving the archypix-worker crate, run in `nix develop`.

# Agents

When making changes to the codebase:

- Don’t over-comment in the code: keep comments concise and only where necessary. Detail important concepts in the doc instead, and don’t repeat them
  in detail in the comments.
- Keep documentation up to date. Match the level of detail already present — do not add overly specific descriptions of what was changed beyond what
  the rest of the doc covers.
- When editing the api, update `06_API_REFERENCE.md`.
- When completing a task, update `99_ROADMAP_MVP.md`, and eventually add things not implemented yet into it.
- On back, keep tests up to date. New features and modified behaviour should be reflected in the test suite.
- On front, don’t start or preview the frontend server yourself. Only check that it builds. The user can give you feedback on the frontend
  changes.
