# Archypix — Backend

The authoritative per-instance API server for Archypix. It owns user accounts, pictures, tags, and shares for one deployment, and communicates with
other backends through the federation protocol.

For a full overview of the project, see the [root README](https://github.com/ClementGre/Archypix).

## Tech stack

| Concern            | Crate                                                                                                          |
|--------------------|----------------------------------------------------------------------------------------------------------------|
| HTTP server        | [Axum](https://github.com/tokio-rs/axum) + [Tokio](https://tokio.rs/)                                          |
| Database           | [SQLx](https://github.com/launchbadis/sqlx) + PostgreSQL (compile-time checked queries)                        |
| Session cache      | [bb8-redis](https://github.com/djc/bb8) + Redis                                                                |
| Object storage     | [aws-sdk-s3](https://github.com/awslabs/aws-sdk-rust) (MinIO / AWS S3)                                         |
| Auth               | [jsonwebtoken](https://github.com/Keats/jsonwebtoken), [argon2](https://github.com/RustCrypto/password-hashes) |
| Federation HTTP    | [reqwest](https://github.com/seanmonstar/reqwest)                                                              |
| Structured logging | [tracing](https://github.com/tokio-rs/tracing) + tracing-subscriber                                            |

## Configuration

Copy `.env.example` to `.env`. The file contains minimal configuration. The rest of the configuration can be done through the admin dashboard, or
adding more env variables.

```bash
cp .env.example .env
```

A few key concepts to be aware of:

- **`BACK_DOMAIN`** is the public domain of this specific backend instance (e.g. `backend1.example.com`), used as the JWT audience.
- **`GLOBAL_DOMAIN`** is the shared identity domain that appears in user handles (`@user:example.com`). It can differ from `BACK_DOMAIN`, which is
  common when a reverse proxy forwards WebFinger requests from the public domain to this backend.
- **`USE_RESOLVER`**: set to `true` when multiple backends share the same `GLOBAL_DOMAIN` via a Resolver. Set to `false` for a standalone instance.

Log level:
```bash
RUST_LOG=info,archypix_back=debug    # default. SQLx query logs appear at the `sqlx=debug` level.
RUST_LOG=info,archypix_back=trace    # verbose: service calls, cache hits
```

## Building

Prerequisites: Rust (stable, edition 2024) via [rustup](https://rustup.rs/), PostgreSQL, Redis, and an S3-compatible store.

```bash
# Development
cargo run

# Release
cargo build --release
./target/release/archypix-back

# Docker
docker compose up
```

## Database migrations

Migrations in `migrations/` are applied automatically at startup. To run them manually or create new ones:

```bash
sqlx migrate run --database-url "postgres://user:password@host/archypix_back"
sqlx migrate add <migration_name>
```

The `.sqlx/` directory contains cached query metadata for offline builds (CI without a live database). Regenerate it after any SQL change:

```bash
cargo sqlx prepare
```

## Testing

Tests require a live PostgreSQL database (SQLx spins up an isolated schema per test via `#[sqlx::test]`). Set `DATABASE_URL` before running:

```bash
DATABASE_URL="postgres://archypix:archypix@localhost/archypix_back"
(cd back && cargo sqlx migrate run)

cargo test -p archypix-back --lib          # unit tests only
cargo test --test federation               # full federation suite
cargo test --test federation contract      # end-to-end two-server flows only
```

| Location                                   | Kind                | What it covers                                                       |
|--------------------------------------------|---------------------|----------------------------------------------------------------------|
| `src/domain/` (inline)                     | Unit                | Tag manipulation, ltree rules, pipeline evaluator                    |
| `src/repository/` (inline `#[sqlx::test]`) | DB integration      | Repository functions against a real schema                           |
| `tests/services_*.rs`                      | Service integration | Multi-step workflows: auth, users, tags, jobs, shares                |
| `tests/worker_contract.rs`                 | HTTP contract       | Worker claim → complete/fail cycle                                   |
| `tests/federation/contract.rs`             | Federation e2e      | Two real Axum servers on ephemeral ports; real TCP between instances |
| `tests/federation/rejection.rs`            | Security            | Malformed/unauthorised requests into a single in-process router      |
| `tests/federation/presign.rs`              | Presign             | `POST /api/federation/pictures/presign` authorised by `share_token`  |

Shared test helpers live in `tests/common/` (app state, seeds, in-memory cache/storage) and `tests/common/federation.rs` (server spawn, JWT helpers,
cache seeding).

## Code structure

- `domain/` — pure business types and rules, no I/O
- `repository/` — SQL queries only, no business logic
- `services/` — multi-step workflows with explicit transaction boundaries
- `clients/` — outbound HTTP adapters (federation, resolver)
- `infra/` — raw connectivity primitives (settings, DB, Redis, S3, crypto, error)
- `state.rs` — `AppState`: holds all infra handles, no business logic
- `api/` — HTTP handlers, auth extractors, request/response models
  - `middleware/` — JWT extractors for each token type (user, resolver, federation, worker)
    - `user/` — `/api/auth/*`, `/api/public/*`, `/api/authenticated/*`
    - `admin/` — `/api/admin/*`
    - `federation/` — `/api/federation/*`
    - `resolver/` — `/api/resolver/*`
  - `worker/` — `/api/worker/*`
    - `webfinger.rs` — `/.well-known/webfinger` (standalone mode only)

Dependency rule: `api → services → repository → domain`. No layer may reach upward. `clients` and `infra` are horizontal utilities.

## API surface

| Route group        | Base path                      | Auth            |
|--------------------|--------------------------------|-----------------|
| Auth and public    | `/api/auth/*`, `/api/public/*` | None / user JWT |
| Authenticated      | `/api/authenticated/*`         | User JWT        |
| Admin              | `/api/admin/*`                 | Admin User JWT  |
| Resolver callbacks | `/api/resolver/*`              | Resolver JWT    |
| Federation         | `/api/federation/*`            | Federation JWT  |
| Workers            | `/api/worker/*`                | Worker JWT      |
| WebDAV *(planned)* | `/dav/*`                       | User JWT        |

Worker JWT authentication uses the same shared-secret pattern as the Resolver: `WORKER_JWT_SECRET` must be set identically on the backend and all
worker instances. Workers generate short-lived tokens (300 s) signed with HS256.

## License

[AGPL-3.0](https://github.com/ClementGre/Archypix/blob/main/LICENSE)
