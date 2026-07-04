# Resolver Architecture

Factual reference for the `archypix-resolver` crate **as it stands today**. The resolver is a small,
stateless-by-design Rust (Axum) service that maps a shared global identity domain to the backend that
owns each user, and routes new-user registration across backends. Read this before touching
`resolver/**`. The planned evolution (fleet admin dashboard, delegation-token auth, smarter selection,
runtime config) is specified in
[`doc/features/23_resolver_admin_and_runtime_config.md`](features/23_resolver_admin_and_runtime_config.md).

## A) Purpose & role

One `GLOBAL_DOMAIN` (the part after `:` in a handle `@user:global_domain`) can be served by many
independently deployed backends. The resolver is the shared front for that domain:

- **WebFinger** — answers `/.well-known/webfinger`, resolving `@user:global_domain` → the owning
  backend's public base URL. Frontends and peer backends use this for discovery.
- **Registration routing** — serves `POST /api/public/register` (the *same* path a standalone backend
  serves, so the frontend uses one URL regardless of topology), picks the least-loaded backend,
  forwards the registration, and records the `username → back_domain` mapping.
- **Backend self-registration** — backends call `POST /api/backends` at startup to advertise their
  `back_domain`, `use_https`, and `internal_url`.
- **Mapping update** — `POST /api/update`, called by a backend when a user migrates instances.

The resolver holds **no** user content and no per-user metadata beyond the username→backend mapping.

## B) Module layout

`module_name.rs` alongside a `module_name/` dir per the workspace convention; the resolver is currently
flat:

| File          | Responsibility                                                                        |
|---------------|---------------------------------------------------------------------------------------|
| `main.rs`     | `AppState`, router wiring, CORS, tracing, startup logging.                            |
| `config.rs`   | `Config::from_env` — env parsing + Postgres URL builders.                             |
| `database.rs` | `init_database` (schema bootstrap) + all SQL helper functions.                        |
| `handler.rs`  | All HTTP handlers, the WebFinger/JRD encoding, and the JWT helpers.                   |
| `error.rs`    | `AppError` enum + `IntoResponse` (JSON `{ "error": … }`), `anyhow::Error` conversion. |

## C) `AppState`

```rust
struct AppState {
    db: PgPool,
    cache: moka::future::Cache<String, String>, // username → backend public URL
    global_domain: String,
    resolver_jwt_secret: String,
    reqwest_client: reqwest::Client,
}
```

## D) Configuration (env)

| Field                 | Env var                                                         | Default                                       | Notes                                |
|-----------------------|-----------------------------------------------------------------|-----------------------------------------------|--------------------------------------|
| `listen_addr`         | `LISTEN_ADDR`                                                   | `0.0.0.0:80`                                  |                                      |
| DB (split)            | `DB_HOST` (req), `DB_PORT`, `DB_USER`, `DB_PASSWORD`, `DB_NAME` | `5432` / `postgres` / — / `archypix_resolver` | Assembled into a `postgres://` URL.  |
| `global_domain`       | `GLOBAL_DOMAIN` (req)                                           | —                                             | Must match every registered backend. |
| `resolver_jwt_secret` | `RESOLVER_JWT_SECRET` (req)                                     | —                                             | Shared HS256 secret with backends.   |
| `cors_origins`        | `CORS_ORIGINS` (req)                                            | —                                             | Comma list; `*` = any (dev only).    |
| `cache_ttl_secs`      | `CACHE_TTL_SECS`                                                | `3600`                                        | Mapping cache TTL.                   |
| `cache_max_capacity`  | `CACHE_MAX_CAPACITY`                                            | `100000`                                      | Mapping cache size.                  |

## E) Database schema

Bootstrapped idempotently by `init_database` (`CREATE TABLE IF NOT EXISTS`) — **not** SQLx migrations
today.

```sql
backends (
  back_domain  VARCHAR(255) PRIMARY KEY,   -- public host[:port]
  use_https    BOOLEAN NOT NULL,           -- scheme for the public URL
  internal_url VARCHAR(255) NOT NULL,      -- how the resolver reaches this backend
  created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
)

user_mappings (
  username    VARCHAR(255) PRIMARY KEY,
  back_domain VARCHAR(255) NOT NULL REFERENCES backends(back_domain),
  created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
)  -- + INDEX on back_domain
```

Helpers (`database.rs`): `get_backend_url` (join → `scheme://back_domain`), `upsert_mapping`,
`upsert_backend`, `list_backends`, `count_users_per_backend` (→ `(back_domain, use_https, internal_url,
user_count)` ordered by `user_count ASC`), `username_exists`.

## F) HTTP endpoints (`handler.rs`)

| Route                    | Method | Auth         | Behaviour                                                                 |
|--------------------------|--------|--------------|---------------------------------------------------------------------------|
| `/.well-known/webfinger` | GET    | none         | Parse `archypix:@user:domain`, cache→DB lookup, return JRD.               |
| `/api/public/register`   | POST   | none         | Validate, reject taken username, pick least-loaded backend, forward, map. |
| `/api/backends`          | POST   | resolver JWT | Backend self-registration (`upsert_backend`).                             |
| `/api/backends`          | GET    | resolver JWT | List backend domains.                                                     |
| `/api/update`            | POST   | resolver JWT | `upsert_mapping` + cache invalidate (user migration).                     |
| `/health`                | GET    | none         | `{ status, service }`.                                                    |

**WebFinger.** Resource form `archypix:@<user>:<domain>`; `<domain>` must equal `GLOBAL_DOMAIN`.
`splitn(2, ':')` keeps a `host:port` domain intact. Cache-then-DB; on hit inserts into the cache;
unknown user → `404`. Response is `application/jrd+json` with a single link `rel="backend_url"`.

**Registration.** Rejects an empty/taken username, requires at least one registered backend
(`503` otherwise), selects `count_users_per_backend[0]` (fewest users), mints a short-lived resolver JWT
scoped to that backend, `POST`s `{internal_url}/api/resolver/users` with it, and on success
`upsert_mapping`s the user. A backend error is propagated as `BackendError(status, body)`.

## G) JWT authentication (current)

Symmetric HS256 with the shared `RESOLVER_JWT_SECRET`, `token_type = "resolver"`.

- **Inbound** (`/api/backends`, `/api/update`): `verify_resolver_jwt` — decode with the shared secret,
  require `aud == GLOBAL_DOMAIN` and `token_type == "resolver"`.
- **Outbound** (registration forwarding): `generate_resolver_jwt` — `aud = back_domain`,
  `iss = "resolver"`, `token_type = "resolver"`, 300 s TTL, random `jti`. The target backend verifies it
  with the same shared secret.

Claims mirror the backend's `JwtClaims` (`sub`, `is_admin`, `instance`, `token_type`, `aud`, `iss`,
`exp`, `iat`, `jti`); the resolver keeps its own local copy of the struct.

## H) Caching, CORS, observability

- **Cache** — `moka::future::Cache<String, String>` (username → public URL), `cache_ttl_secs` TTL,
  `cache_max_capacity` bound. Invalidated on `/api/update`.
- **CORS** — `tower_http::CorsLayer`; `*` in `CORS_ORIGINS` ⇒ allow-any, else an explicit origin list.
- **Tracing** — `tracing_subscriber` env-filter + `TraceLayer`; handlers log with a `user` /
  `token_type` / `source` field vocabulary.

## I) Planned evolution — feature 23

The resolver is slated to become the **fleet admin control plane**: an operator-token dashboard
(`/admin/resolver`), aggregate + per-instance-proxy admin, backend-signed **delegation-token** auth
delivered by a backend **heartbeat** (narrowing `RESOLVER_JWT_SECRET` to authenticating those pushes
only), smarter placement strategies with instance-pinned invites and capacity gates, registration modes
(open / invite / admin-invite), runtime config, and a switch to **SQLx migrations**. This section is a
pointer; the design lives in
[`doc/features/23_resolver_admin_and_runtime_config.md`](features/23_resolver_admin_and_runtime_config.md)
and this document is updated as it ships.
