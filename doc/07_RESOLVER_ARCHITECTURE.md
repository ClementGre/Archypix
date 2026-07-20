# Resolver Architecture

Factual reference for the `archypix-resolver` crate **as it stands today** (feature 23 shipped the
fleet-admin control plane). The resolver maps a shared global identity domain to the backend that owns
each user, routes new-user registration, and is the **fleet admin control plane**. Read this before
touching `resolver/**`. Design rationale + target state:
[`doc/features/23_resolver_admin_and_runtime_config.md`](features/23_resolver_admin_and_runtime_config.md).

## A) Purpose & role

One `GLOBAL_DOMAIN` can be served by many independently-deployed backends. The resolver is the shared
front. Its **entire router is nested under one top-level prefix, `/archypix-resolver/`** (feature 25),
so a self-hoster has a single forwarding rule and there's no `.well-known` collision with other apps on
the apex domain. Handler paths are unchanged inside the mount.

- **Resolution** — `GET /archypix-resolver/resolve?user=&domain=` → owning backend public URL
  (moka-cached), one HTTP call; the federation/login hot path. Resolver behaviour is unchanged by feature 28, but
  note the **caller side**: a backend resolving a peer keeps a long-lived stale copy of a successful
  answer and serves it if the resolver is later unreachable (connection error, not a 404), so a resolver
  blip is non-fatal for already-known peers (feature 28 §4.5); a resolver 404 ("domain is its own
  backend") is deliberately **not** cached.
- **Bootstrap discovery** — `GET /archypix-resolver/info` → `{ is_resolver: true, api_url }`, so the
  frontend learns where the heavier `/api/public/*` + `/api/resolver-admin/*` surface lives and that a
  fleet dashboard exists (feature 25). A standalone backend answers the same route with
  `{ is_resolver: false, api_url: <its public URL> }`. The **backend also serves `resolve`** (returning
  its own public URL if the user exists), so a single-domain deployment can forward
  `/archypix-resolver/` to the backend and resolve without a resolver.
- **Registration routing** — `POST /api/public/register`: enforces the registration mode + invite,
  picks a backend by the configured **selection strategy** (honouring an invite's `instance_pin`),
  forwards the signup to that backend (replaying its delegation token), records the mapping.
- **Backend self-registration + heartbeat** — backends `POST /api/backends` at startup and
  `POST /api/backends/heartbeat` periodically (delivering a fresh delegation token + fleet metrics).
- **Fleet admin dashboard** — native aggregate/self-monitoring endpoints, resolver-own runtime config,
  registration/selection policy, invites, a per-instance proxy to each backend's `/api/admin/*`, and a
  config-matrix across backends, under `/api/resolver-admin/*` (operator-token auth).

The resolver holds **no** user content beyond the `username → backend` mapping and fleet state.

## B) Module layout (`resolver/src/`, mirrors the backend's layers)

| Path                                                                        | Responsibility                                                                                                                                                                                  |
|-----------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `main.rs`                                                                   | Boot: load config, `sqlx::migrate!`, reload DB overrides, seed operator token, spawn routines, dynamic CORS, serve.                                                                             |
| `config.rs`                                                                 | The layered [`Settings`](../common/src/settings.rs) engine: `setting_keys`, `registry()`, `SelectionStrategy`, `database_url`, `load_env_only`/`reload_from_db`. `Config = Arc<Settings>`.      |
| `state.rs`                                                                  | `AppState` — `db`, moka `cache`, `config`, `jwt` (`JwtService`), `backends` (`BackendClient`).                                                                                                  |
| `repository.rs`                                                             | All SQL — compile-time-checked `query!`/`query_as!` (offline `.sqlx` cache): backends+heartbeat state, user_mappings, invites, operator credential, settings overrides.                         |
| `clients/backend.rs`                                                        | `BackendClient` — resolver→backend calls **replaying the stored delegation token**; `register_user`, `get_json`, `proxy_json`.                                                                  |
| `services/{operator,registration,selection}.rs`                             | Operator credential (seed/verify/session/refresh), registration-mode+invite gate, placement strategies.                                                                                         |
| `api.rs` + `api/{bootstrap,public,backends,admin}.rs` + `api/middleware.rs` | Router (nested under `/archypix-resolver`) + handlers + auth extractors (`AuthPush`, `AuthAdmin`). `bootstrap` = `/info` + `/resolve` (feature 25).                                             |
| `routine.rs`                                                                | `StaleBackendPrune`, `InviteCleanup` on `common::routine`.                                                                                                                                      |
| `lib.rs`                                                                    | Re-exports the modules so the binary and `tests/` share one compilation. `AppError` + `IntoResponse` (+ `From` impls) come from [`common::error`](../common/src/error.rs); no local `error.rs`. |

## C) Configuration (feature 23 §4)

The resolver's config is the same `common::settings` engine as the backend — one source of truth, read
via `config.get(setting_keys::X)`. **Core** fields (env-only): DB connection, `GLOBAL_DOMAIN`,
`USE_HTTPS`, `PUBLIC_URL` (the `api_url` advertised by `/archypix-resolver/info` — defaults to
`{scheme}://{GLOBAL_DOMAIN}/archypix-resolver`, feature 25), `RESOLVER_JWT_SECRET`,
`RESOLVER_ADMIN_TOKEN`, `LISTEN_ADDR`, cache TTL/capacity. **Runtime**
(DB-editable from the dashboard, hot): `CORS_ORIGINS`, `SELECTION_STRATEGY`, `STATIC_BACKEND`,
`PIN_IMPORTANCE`, `REGISTRATION_MODE`, `DELEGATION_STALE_SECS`, `STALE_PRUNE_INTERVAL_SECS`,
`INVITE_CLEANUP_INTERVAL_SECS`. Overrides live in `resolver_settings`; `PATCH` rebuilds the snapshot.

## D) Database schema (SQLx migrations, `resolver/migrations/`)

`0001_init` — `backends`, `user_mappings`. `0002_fleet_admin` — adds to `backends`
(`delegation_token`, `delegation_expires_at`, `user_count`/`picture_count`/`storage_bytes`,
`last_heartbeat_at`, `healthy`, `reachable`, `accepting_registrations`, `max_users`, `version`,
`last_selected_at`); `resolver_settings`, `invites` (with `instance_pin`, `created_by`),
`resolver_admin` (single-row hashed operator token + auto-rotating refresh).

## E) Auth (feature 23 §3)

- **`Resolver` push token** (shared `RESOLVER_JWT_SECRET`, verified by `AuthPush`) — backend→resolver
  pushes only (self-register / update / heartbeat).
- **`ResolverDelegation`** (backend-signed, delivered by the heartbeat) — the resolver **replays** it
  on every call to that backend (`BackendClient`); the resolver never mints a token a backend trusts. A
  backend with no live token (never/stale heartbeat) is **unreachable** → `503`.
- **`ResolverAdminSession`** (resolver-signed, verified by `AuthAdmin`) — the operator dashboard. The
  operator token (`RESOLVER_ADMIN_TOKEN`, argon2-hashed in `resolver_admin`, generated + printed once if
  unset) exchanges for a short session JWT + a 1-month **auto-rotating** refresh token.

## F) Selection strategies (feature 23 §7)

`SELECTION_STRATEGY`: `least_users` (default), `least_pictures`, `least_storage`, `round_robin`,
`static`. Metric strategies read the **heartbeat metrics** (authoritative over the drift-prone mapping
count). Capacity (`accepting_registrations`, `max_users`) + reachability are **hard** gates. An
invite's `instance_pin` is honoured per `pin_importance`: metric strategies honour it iff
`metric(pinned) − min(others) ≤ pin_importance`; round-robin/static honour it iff `pin_importance ≥ 1`.
`round_robin` picks the least-recently-selected eligible backend (`last_selected_at`).

## G) Endpoints

Every route below is served under the **`/archypix-resolver/`** mount prefix (feature 25) — e.g.
`/archypix-resolver/api/public/register`, `/archypix-resolver/health`.

| Route                                                       | Auth                      | Behaviour                                                                                 |
|-------------------------------------------------------------|---------------------------|-------------------------------------------------------------------------------------------|
| `GET /info`                                                 | none                      | Bootstrap discovery: `{ is_resolver: true, api_url }` (feature 25).                       |
| `GET /resolve?user=&domain=`                                | none                      | `@user:domain` → `{ backend_url }` (moka-cached); `404` for unknown user / mismatch.      |
| `POST /api/public/register`                                 | none                      | Mode+invite gate → strategy pick → forward (delegation) → map.                            |
| `GET /api/public/registration-info`                         | none                      | `{ mode }` — lets register/profile UIs adapt.                                             |
| `GET /api/public/invites/{code}`                            | none                      | `{ valid, invited_by }` invite preview for the register page.                             |
| `POST /api/update`                                          | `AuthPush`                | Update `username→backend` mapping + invalidate cache.                                     |
| `POST /api/backends` / `GET`                                | `AuthPush`                | Self-register / list backend domains.                                                     |
| `POST /api/backends/heartbeat`                              | `AuthPush`                | Store delegation token + metrics, mark reachable.                                         |
| `POST\|GET /api/backends/invites`, `DELETE …/{code}`        | `AuthPush`                | Backend-driven invite CRUD (user mints on their backend, pushed up; `created_by` filter). |
| `POST /api/resolver-admin/login` \| `/refresh`              | operator token \| refresh | → session + rotating refresh.                                                             |
| `GET /api/resolver-admin/overview` \| `/backends`           | `AuthAdmin`               | Fleet Σ + per-backend state rows.                                                         |
| `GET /api/resolver-admin/next-backend`                      | `AuthAdmin`               | Dry-run placement: `{ back_domain }` where the next signup lands (null if none eligible). |
| `GET\|PATCH /api/resolver-admin/settings`, `DELETE …/{key}` | `AuthAdmin`               | Resolver's own runtime config.                                                            |
| `GET /api/resolver-admin/routines`, `POST …/{name}/trigger` | `AuthAdmin`               | Resolver routine status (prune/cleanup) + manual trigger.                                 |
| `GET\|POST /api/resolver-admin/invites`, `DELETE …/{code}`  | `AuthAdmin`               | Invite CRUD (with `instance_pin`).                                                        |
| `PATCH /api/resolver-admin/backends/{d}/capacity`           | `AuthAdmin`               | Per-backend accepting/max_users.                                                          |
| `GET /api/resolver-admin/config-matrix`                     | `AuthAdmin`               | Fan-out `GET /api/admin/settings` across backends.                                        |
| `ANY /api/resolver-admin/instances/{d}/api/admin/{*path}`   | `AuthAdmin`               | Delegation-replay proxy.                                                                  |
| `GET /health`                                               | none                      | `{ status, service }`.                                                                    |

## H) Routines (feature 23 §8.3)

`StaleBackendPrune` (interval `stale_prune_interval_secs`, startup) marks a backend unreachable once
its stored delegation token is past expiry. `InviteCleanup` (interval `invite_cleanup_interval_secs`)
deletes expired / fully-consumed invites. **`MappingReconcile`** (interval
`mapping_reconcile_interval_secs`, startup, feature 24) reconciles `username → backend` mappings against
each **reachable** backend's authoritative `/api/admin/users` list (delegation replay): it adds/fixes a
mapping a backend claims and **prunes** one only when its own backend was reachable+queried and the user
is absent from every reachable backend (users on an unreachable backend are never pruned). Cache
coherence is left to the moka TTL. All three on `common::routine`, spawned with a `RoutineStatus` handle
collected into `AppState.routine_registry` so the dashboard's `GET /api/resolver-admin/routines` reports
live status + can trigger them.

**Invite codes & semantics** (`common::registration`): a code is 9 lowercase base36 chars
(`generate_invite_code`), shown grouped (`ABC-DEF-GHI`). `max_uses` is `Some(n>0)` capped, `Some(0)`
uncapped invitation, `None` a **tracking referral link** — unlimited but redeemable only in `open` mode
(inactive/tombstoned otherwise). Redemption is atomic in SQL; the mode gate is in `authorize_registration`.

## I) Testing

Integration tests live in `resolver/tests/resolver.rs` (ephemeral DB per test via `#[sqlx::test]`,
online SQLx against `DATABASE_URL` — the resolver ships an offline `.sqlx` cache so `SQLX_OFFLINE`
builds work): heartbeat/reachability + stale-prune, the five selection strategies incl. the pin-delta
boundary and hard capacity gates, registration modes + invite max-uses/expiry atomicity, the operator
credential (seed/login/refresh rotation), settings-engine env-lock precedence, and the
delegation-replay client's unreachable-backend behaviour.

## J) Not yet implemented

Only the frontend `/admin/resolver` dashboard (feature file to be written; frontend built separately).
Everything backend-side — including config-matrix PATCH fan-out — has shipped.
