# Feature 23 — Resolver admin dashboard, runtime config & registration rules

Umbrella feature covering three coupled roadmap items — **Resolver's admin dashboard**,
**Registration rules**, and **Admin config instead of envs** — plus the cross-cutting refactors they
require (a shared JWT service, a shared runtime-config engine, and lifting the routine framework into
`archypix-common`). The resolver's *current* architecture is documented factually in
[`doc/07_RESOLVER_ARCHITECTURE.md`](../07_RESOLVER_ARCHITECTURE.md); this file is the design rationale
and the target state.

## 1. Overview & goals

When several backends share one global domain behind a resolver, an operator today can only administer
**one instance at a time** (the admin logs into a specific backend). This feature:

- Lets the **frontend use the resolver as the admin dashboard** for the whole fleet — an aggregate
  cross-instance view plus a thin per-instance proxy to each backend's existing `/api/admin/*`.
- Makes the **resolver smarter**: pluggable new-user placement strategies (least users / pictures /
  storage, round-robin, static), capacity gates, and registration rules (open / invite / admin-invite)
  with instance-pinned invites.
- Moves most **configuration out of env vars** into a runtime layer editable from the dashboard, while
  keeping bootstrap/secret config env-only.
- Hardens **resolver↔backend auth**: the backend becomes the sole token authority; the resolver only
  ever *replays* short-lived, backend-signed tokens it receives over a heartbeat.

**Preserved invariant:** a backend deployed **without** a resolver (homelab / single instance) keeps
working standalone — it runs the same registration domain logic locally and exposes the same runtime
config on its own `/admin`.

## 2. Decisions

1. **Backend is the token authority.** Every resolver→backend request carries a **backend-signed**
   delegation JWT (signed with the backend's own `JWT_SECRET`), delivered to the resolver by a periodic
   heartbeat. The resolver never mints a token a backend will accept.
2. **`RESOLVER_JWT_SECRET` demoted to one direction.** It authenticates the backend's *pushes to the
   resolver* only (self-register / mapping-update / heartbeat). The backend no longer uses it to
   authenticate the resolver.
3. **Hybrid dashboard.** The resolver owns *native* endpoints for aggregate views, self-monitoring, and
   resolver-only policy (registration, selection, invites); a per-instance subpath *thin-proxies* to a
   backend's `/api/admin/*`. Per-backend config is edited by **fan-out** — the resolver stores no
   backend config, it issues N requests and shows the diff.
4. **Registration policy is resolver-side in resolver mode.** Modes, codes, capacity gates and invites
   live on the resolver; the backend is *dumb* (accepts every resolver-forwarded signup, no check). The
   same domain logic runs *locally* on a standalone backend.
5. **Config = core (env-only) + runtime (layered).** Core secrets/topology stay env-only. Everything
   operational is `default → env(locked) → DB override`, `ArcSwap`-hot-swapped, edited from the
   dashboard. An **env-set value locks the field** (shown read-only in the UI).
6. **Three lifts into `common`:** the `JwtService`/claims, the registration domain logic, and the
   generic routine framework — each already duplicated or needed by ≥2 crates.
7. **Resolver adopts SQLx migrations**, replacing the ad-hoc `init_database` `CREATE TABLE IF NOT
   EXISTS` bootstrap.

---

## 3. Auth model — backend-issued delegation tokens

### 3.1 Token taxonomy

| Token                  | Signed with                  | Verified by  | Carries                                                  | Used for                                                       |
|------------------------|------------------------------|--------------|----------------------------------------------------------|----------------------------------------------------------------|
| `Resolver` (push)      | shared `RESOLVER_JWT_SECRET` | resolver     | `iss=back_domain`                                        | backend→resolver: self-register, mapping update, **heartbeat** |
| `ResolverDelegation`   | backend `JWT_SECRET`         | that backend | `is_admin=true`, `iss=aud=back_domain`, `sub="resolver"` | **all** resolver→backend calls (provisioning, admin, config)   |
| `ResolverAdminSession` | resolver admin secret        | resolver     | operator session                                         | the dashboard frontend → resolver native endpoints             |

The old symmetric "resolver signs, backend verifies with the shared secret" path is **removed**. The
shared secret now flows in one direction only (row 1).

### 3.2 Heartbeat & token rotation

Backend routine `ResolverHeartbeat` (§8.2), interval `resolver_heartbeat_interval_secs` (default 300):

1. Mint a fresh `ResolverDelegation` token: backend-signed, `is_admin=true`, TTL
   `resolver_delegation_ttl_secs` (default 360 — deliberately > the interval so there's always overlap).
2. `POST {resolver}/api/backends/heartbeat` (authed by a `Resolver` push token, §3.1 row 1) carrying
   `{ delegation_token, user_count, picture_count, storage_bytes, healthy, version }`.
3. The resolver stores the token + metrics on the backend's state row (§10.2) and marks it reachable.

The resolver **replays** `delegation_token` (as `Authorization: Bearer …`) on every call it makes to
that backend. Because the TTL is ~6 min and refreshed every ~5, the resolver always holds a live one; a
compromised resolver has **≤ one TTL** of replay and **cannot forge** a new token.

`self_register` (startup) is unchanged except it no longer needs to reach the backend afterward — the
first heartbeat delivers the first delegation token. Until then the backend is **unreachable** (§7.3).

### 3.3 Backend `AuthAdmin` / `AuthResolver` changes

- `AuthAdmin` accepts **either** a user token with `is_admin` (direct login on `/admin`) **or** a valid
  `ResolverDelegation` (via the resolver proxy). Same guard, two issuers.
- `/api/resolver/*` (provisioning) switches from the shared-secret `Resolver` token to the
  `ResolverDelegation` token — one backend-signed credential authorizes everything the resolver does.
- There is **no resolver identity** on the backend: a proxied admin action is attributed to
  `sub="resolver"`. The operator's dashboard identity is not forwarded (§5.1).

### 3.4 Security properties

- A leaked backend `JWT_SECRET` was already game-over for that backend; nothing new is exposed.
- A leaked `RESOLVER_JWT_SECRET` now only lets an attacker *impersonate a backend pushing to the
  resolver* (poison metrics / register a rogue backend) — it can no longer mint anything a backend
  trusts. Rogue-backend registration is gated by the resolver operator approving backends (out-of-band).
- A compromised resolver: ≤ 1 delegation-TTL of admin replay per backend; no token forgery; can still
  provision/repoint (intrinsic to its role).

---

## 4. Runtime configuration system

### 4.1 Core (env-only) vs runtime (layered)

**Core — never runtime-editable** (bootstrap, secrets, identity, topology): DB/Redis connection, all S3
endpoints + credentials, `JWT_SECRET` / `WORKER_JWT_SECRET` / `RESOLVER_JWT_SECRET`, resolver admin
secret, `LISTEN_ADDR`, `BACK_DOMAIN` / `GLOBAL_DOMAIN` / `BACK_USE_HTTPS`, `USE_RESOLVER`,
`RESOLVER_INTERNAL_URL` / `BACK_INTERNAL_URL`, `CONFIG_FILE`-style bootstrap. These stay on the existing
immutable [`Config`](../../back/src/infra/settings.rs).

**Runtime — layered & dashboard-editable:** retention days, all rate-limit + pending-share caps, all
routine intervals/batches (pipeline/job/purge/exif-drain), `default_storage_quota_bytes`, trace
propagation peers, **CORS origins**, and (resolver) registration mode / selection strategy / pin
importance / per-backend capacity.

### 4.2 Layering & precedence

```
built-in default  <  env var (LOCKS the field)  <  DB runtime override
```

- If a runtime field's env var **is set**, that value wins and the field is **locked**: the dashboard
  renders it read-only with a "defined by environment" badge, and PATCH is rejected. This keeps
  infra-as-code deployments deterministic.
- If the env var is unset, the field is default until an admin sets a DB override; the override wins and
  is editable/clearable.
- **No config-file layer** for now (env + DB only).

### 4.3 The settings engine (`common::settings`)

A small generic engine shared by backend and resolver:

- Each crate declares its runtime fields (name, type, default, `restart_required: bool`).
- `Settings::load(db)` merges `default → env → db_overrides` into a snapshot; env-locked fields recorded
  so the API can flag them.
- Held as `Arc<ArcSwap<Snapshot>>` in `AppState`. Read is a cheap `load()`; a PATCH writes the DB row and
  rebuilds the snapshot (single `ArcSwap::store`), so handlers see changes on the next read.
- Typed accessors (`settings.rate_limit_login_max()`, …). The immutable `Config` keeps core accessors.

### 4.4 Hot-reload & restart-required

- **Hot** (most fields): handlers read the live snapshot each request.
- **Routine intervals:** routines are given the settings handle (not a copied `Duration`) and read the
  interval from the snapshot **at each tick** — a change takes effect after the current wait ends. (No
  routine re-spawn.)
- **CORS:** replaced by a dynamic middleware that reads allowed origins from the snapshot per request
  (the static `tower_http::CorsLayer` can't change post-build).
- **Restart-required fields** (`LISTEN_ADDR`-adjacent, anything consumed only at startup) are tagged
  `restart_required` and the UI shows "takes effect after restart".

### 4.5 / 4.6 Config surfaces

The **backend** exposes `GET/PATCH /api/admin/settings` (each field: value, source `default|env|db`,
`locked`, `restart_required`). The **resolver** exposes the analogous `GET/PATCH
/api/resolver-admin/settings` for its *own* config. Backend per-instance config is reached from the
resolver **only by proxy/fan-out** (§5.3–5.4), never stored on the resolver.

---

## 5. Resolver admin dashboard

### 5.1 Dashboard auth (operator token → session)

- A single **operator token** (root-password style). Env `RESOLVER_ADMIN_TOKEN` sets it; it may be a
  bcrypt/argon2 **hash** or a **plaintext** value (a startup warning is logged for plaintext). If unset,
  the resolver **generates** one at startup and prints it **once** to the console. The token is stored
  **hashed** (argon2) in the resolver DB; it is rotatable from the dashboard — unless env-set, in which
  case the env-lock rule (§4.2) disables rotation.
- Login: the operator presents the token → the resolver issues a short-lived `ResolverAdminSession` JWT
  (signed with the resolver admin secret) the frontend uses as bearer. **No user accounts, no
  identity** — the token *is* the credential.
- The frontend mounts this at **`/admin/resolver`**, which authenticates against the resolver instead of
  any backend's user auth. The existing per-instance `/admin` (backend user-auth) is unchanged and still
  works for direct single-instance administration.

### 5.2 Native endpoints (aggregate + self-monitoring)

Resolver-owned, `ResolverAdminSession`-guarded (`/api/resolver-admin/*`):

- `GET /overview` — fleet totals (Σ users/pictures/storage from stored heartbeat metrics), backend
  list with health / last-heartbeat / reachable, mapping counts, registration stats.
- `GET /backends` / self-monitoring — per-backend state rows (§10.2), including reachability and the
  reason (no heartbeat / delegation expired).
- Registration policy CRUD (§6), selection-strategy + `pin_importance` config, per-backend capacity
  (`accepting_registrations`, `max_users`), invite CRUD.
- `GET/PATCH /settings` — the resolver's own runtime config (§4.6).

### 5.3 Per-instance thin proxy

`ANY /api/resolver-admin/instances/{back_domain}/api/admin/*path` → reverse-proxy to that backend's
`/api/admin/*path`, injecting the stored `ResolverDelegation` bearer, streaming the response back. This
exposes the **entire** existing backend admin surface through the resolver with **zero duplication**. A
backend marked unreachable returns `503` from the proxy.

### 5.4 Config fan-out & diff view

- **Read/diff:** `GET /api/resolver-admin/config-matrix` fans out `GET /api/admin/settings` to every
  reachable backend and returns, per field, the set of distinct values across backends (+ which
  backends hold each). The UI renders a matrix highlighting divergence.
- **Write:** `PATCH /api/resolver-admin/config-matrix` with a field + value + target set (`all` or a
  backend list) fans out `PATCH /api/admin/settings`. **Best-effort**: returns a per-backend
  success/failure list; a locked/failed field on one backend doesn't abort the others.

---

## 6. Registration rules

### 6.1 Modes (`common::registration`)

`RegistrationMode { Open, Invite, AdminInvite }`, an env-lockable runtime field:

- `Open` — anyone registers, no invite. Invites are still *mintable* (for instance-pinning, §7).
- `Invite` — a valid invite is required; **any existing user** may mint invites.
- `AdminInvite` — a valid invite is required; **only admins** may mint invites.

The same enum + invite validation run on the resolver (multi-instance) and on a standalone backend. In
**resolver mode the resolver enforces** the mode; the backend accepts every resolver-forwarded signup
without checking (§2.4).

### 6.2 Invites

Shared `common::registration::Invite` type: `code`, `max_uses` (nullable = unlimited), `uses`,
`expires_at` (nullable), `created_by` (username), `instance_pin` (nullable — **resolver-only**, unused
by standalone backends). Redemption atomically checks validity and increments `uses` at register time.
Invite *links* are just a `code` embedded in a frontend URL; the frontend registers against the resolver
(or the backend, standalone) carrying the code.

- **Standalone:** minted and stored on the backend; no `instance_pin`.
- **Resolver mode:** stored in the resolver `invites` table; minted on the dashboard, or minted on a
  backend and **pushed up** to the resolver. Only resolver invites carry `instance_pin`.

### 6.3 `invited_by` & the invitation graph

New backend column `users.invited_by` (nullable username string, e.g. `alice`; the global domain is
implicit). On resolver-driven signup the resolver supplies the inviter (from the redeemed invite's
`created_by`). A small **user-facing** endpoint + dashboard view renders the invitation graph (who I
invited / who invited me).

### 6.4 Standalone vs resolver enforcement

| Mode          | Standalone backend             | Resolver deployment                                  |
|---------------|--------------------------------|------------------------------------------------------|
| policy source | backend runtime config         | resolver runtime config                              |
| enforcement   | backend `/api/public/register` | resolver `/api/public/register`; backend accepts all |
| invite store  | backend                        | resolver (backend-minted invites pushed up)          |

---

## 7. Instance-selection strategies

### 7.1 Strategies

Resolver runtime field `selection_strategy`: `LeastUsers` (default, current behaviour), `LeastPictures`,
`LeastStorage`, `RoundRobin`, `Static` (a configured pinned backend). Metric strategies read the
**heartbeat metrics** (§3.2, §10.2) — authoritative over the drift-prone `user_mappings` count.

### 7.2 Pin-delta algorithm

An invite's `instance_pin` is a **suggestion** weighted by one resolver field `pin_importance`:

- **Metric strategies:** honour the pin iff `metric(pinned) − min(metric(others)) ≤ pin_importance`
  (delta in the metric's own units — pictures or users or bytes). Otherwise pick the metric-best.
  *Example (metric = pictures): pinned=1000, best-other=900 → Δ=100; `pin_importance ≥ 100` keeps the
  user on the pinned instance.*
- **`RoundRobin` / `Static`:** `pin_importance ≥ 1` ⇒ follow the pin; `0` ⇒ ignore it.

### 7.3 Capacity gates & reachability (hard)

`accepting_registrations` (bool) and `max_users` (nullable) are **resolver** config per backend (not
heartbeat-reported). An instance is eligible only if reachable **and** accepting **and**
`user_count < max_users`. These are **hard** — a full/closed/unreachable instance is never chosen, even
when pinned. If **no** instance is eligible, registration returns an error (`503`). All backends start
**unreachable** until their first heartbeat; a backend goes unreachable when its stored delegation token
would be expired (stale-prune routine, §8.3).

---

## 8. Routine framework lift to `common`

### 8.1 What moves

The generic core of [`back/src/infra/routine.rs`](../../back/src/infra/routine.rs) — the `Routine`
trait, `RoutineHandle`, `Scheduler`/`RunState`/`Phase`, `spawn`, `run_routine`, `run_once`, and the
framework unit tests — moves to `common::routine`, gated behind a `routine` cargo feature (pulls
`tokio`, `tracing`, `async-trait`, `anyhow`, `uuid` only for consumers that enable it). The concrete
backend routines stay in `back/`.

### 8.2 Backend (unchanged + one new routine)

Existing routines are untouched except that they read intervals from the settings snapshot (§4.4).
**New:** `ResolverHeartbeat` (interval `resolver_heartbeat_interval_secs`, `run_on_startup`) mints the
delegation token, gathers metrics, and pushes to the resolver (§3.2). Trigger-only nothing; a missed
heartbeat self-heals on the next tick.

### 8.3 Resolver routines

The resolver enables the `routine` feature and runs:

- **`StaleBackendPrune`** — marks a backend unreachable once its stored delegation token is past expiry
  (threshold = the delegation TTL). Interval a fraction of the TTL.
- **`InviteExpiryCleanup`** — deletes expired / fully-consumed invites on an interval.

The username→backend cache stays on moka's own TTL (no routine).

### 8.4 Worker — deliberately **not** migrated

The worker's job loop ([`worker/src/jobs.rs`](../../worker/src/jobs.rs)) is a per-backend *pull* loop
where a single backend saturates many slots bounded by a **global** semaphore. That is the inverse of
the framework's **per-key-serial, parallel-across-keys** model; force-fitting it would repurpose the
framework's semaphore for job-execution concurrency and lose the `5×` error backoff. The loop stays as
is. (The framework could later host worker *housekeeping* — token pre-refresh, graceful drain — which
the worker lacks today.)

---

## 9. JWT & registration domain-logic lift to `common`

- **JWT:** `JwtService` + `JwtClaims` + `TokenType` move to `common::auth` (feature-gated on
  `jsonwebtoken`), unifying the three current hand-rolled copies — backend
  [`crypto.rs`](../../back/src/infra/crypto.rs), worker
  [`auth.rs`](../../worker/src/auth.rs) (`WorkerClaims`), resolver
  [`handler.rs`](../../resolver/src/handler.rs) (`ResolverJwtClaims`). `TokenType` gains
  `ResolverDelegation` and `ResolverAdminSession`.
- **Registration:** `RegistrationMode`, `Invite`, and validation/redemption logic move to
  `common::registration`, used identically by the resolver and by a standalone backend.
- **Error:** `AppError` (+ `map_sqlx_error`, the `From<AuthError>`/`From<SettingsError>`/
  `From<sqlx::Error>`/`From<anyhow::Error>` conversions) moves to `common::error` (feature-gated
  `error`, with `sqlx`/`auth`/`settings` further gating the conversions that need those types),
  unifying the backend's and resolver's near-identical hand-rolled enums. Both crates' `infra/error.rs`
  / `error.rs` are now thin re-export shims.

---

## 10. Schema changes

### 10.1 Backend migration (`00xx_registration_and_settings`)

- `ALTER TABLE users ADD COLUMN invited_by VARCHAR(255)` (nullable username).
- `CREATE TABLE app_settings (key TEXT PRIMARY KEY, value JSONB NOT NULL, updated_at TIMESTAMPTZ)` — DB
  runtime overrides (§4.3).
- `CREATE TABLE invites (…)` — standalone invite store (`common::registration::Invite` shape, no
  `instance_pin` used).
- Regenerate `schema.sql` + `cargo sqlx prepare` per [03 §I](../03_BACKEND_ARCHITECTURE.md).

### 10.2 Resolver migrations (new SQLx workflow, §2.7)

- Convert existing `init_database` tables (`backends`, `user_mappings`) into `0001_init`.
- `backend_state` (or columns on `backends`): `delegation_token TEXT`, `delegation_expires_at`,
  `user_count`, `picture_count`, `storage_bytes`, `last_heartbeat_at`, `healthy`, `reachable`,
  `accepting_registrations BOOL DEFAULT true`, `max_users BIGINT NULL`.
- `resolver_settings` (key/value JSONB) — resolver runtime config.
- `invites` — with `instance_pin` and `created_by`.
- `resolver_admin` — the hashed operator token (single row) + rotation timestamp.

---

## 11. API surface (full spec → `06_API_REFERENCE.md`)

- **Backend:** `GET/PATCH /api/admin/settings`; registration mode / invite endpoints under
  `/api/admin/*`; `/api/public/register` gains invite-code handling (standalone). `/api/resolver/*`
  re-authed to `ResolverDelegation`. `POST /api/backends/heartbeat` **consumer** is the resolver.
- **Resolver:** `POST /api/resolver-admin/login`; `/api/resolver-admin/{overview,backends,settings,
  registration,selection,invites}`; `/api/resolver-admin/instances/{back_domain}/…` proxy;
  `/api/resolver-admin/config-matrix`; `POST /api/backends/heartbeat`. `/api/public/register` gains
  mode + invite + selection + pin logic.

## 12. Frontend

- `/admin` — unchanged single-instance dashboard (backend user-auth).
- `/admin/resolver` — new dashboard using resolver auth: login with operator token, fleet overview,
  per-instance drill-down (proxy), config matrix with diff + set-all, registration/invite/selection
  management, per-backend capacity. Env-locked fields render greyed with a badge; restart-required
  fields show a notice.
- Settings panel on the single-instance `/admin` reusing the same field metadata.
- User-facing invitation-graph view.

## 13. Edge cases

- **Bootstrap:** backend unreachable until first heartbeat; registration routes only to reachable
  instances.
- **Delegation drift/clock skew:** TTL (360) > interval (300) gives a full overlap window; a late
  heartbeat just re-marks reachable.
- **Env-locked field PATCH:** rejected with a clear "defined by environment" error.
- **Fan-out partial failure:** reported per backend; never atomic across the fleet.
- **All instances full/closed:** `503` on register with an operator-facing reason.
- **Pin vs capacity:** capacity/reachability always win over a pin.
- **Standalone → resolver migration:** local invites don't carry pins; document the switch.
- **Rogue backend heartbeat** (leaked `RESOLVER_JWT_SECRET`): mitigated by operator-approved backend
  registration; metrics from unknown backends are ignored.

## 14. Testing

- Delegation: backend mints/verifies its own `ResolverDelegation`; `AuthAdmin` accepts it and a user
  admin token; rejects an expired one; resolver replay works end-to-end.
- Heartbeat: metrics + token stored; stale-prune flips reachability at TTL; startup-unreachable.
- Settings engine: precedence (default/env-lock/db), env-locked PATCH rejected, hot-swap visible on next
  read, routine picks up a changed interval after its wait, CORS middleware honours a live change.
- Registration: each mode on resolver + standalone; who-can-mint; invite max-uses/expiry atomicity;
  `invited_by` populated (local + resolver-supplied).
- Selection: each strategy; pin-delta boundary (`Δ == importance`); round-robin/static 0/1; capacity +
  all-full error.
- Proxy + config-matrix: proxy injects delegation + streams; matrix diff + fan-out partial failure.
- Resolver SQLx migrations apply cleanly; framework tests move with `common::routine`.

## 15. Doc updates

- **07_RESOLVER_ARCHITECTURE.md** — fold the target state in as it ships (auth, heartbeat, admin API,
  selection, migrations).
- **02_INFRASTRUCTURE_DESIGN.md** — resolver roles: heartbeat, admin proxy, delegation-token auth,
  selection strategies; note `RESOLVER_JWT_SECRET`'s narrowed role.
- **03_BACKEND_ARCHITECTURE.md** — `common::routine` / `common::auth` / `common::settings`; the
  runtime-config split; `AuthAdmin` dual issuer; `ResolverHeartbeat` routine.
- **06_API_REFERENCE.md** — all new endpoints (§11).
- **22_storage_quotas.md** — `default_storage_quota_bytes` becomes a runtime setting; §9 resolver seed
  aligns with this delegation model.
- **99_ROADMAP_MVP.md** — mark the three items in progress → done.

## 16. Work breakdown

1. `common`: lift `JwtService`/claims (`common::auth`), routine framework (`common::routine`, feature),
   `settings` engine, `registration` domain logic. Add `TokenType` variants.
2. Backend auth: `ResolverDelegation` mint/verify, `AuthAdmin` dual issuer, re-auth `/api/resolver/*`,
   `ResolverHeartbeat` routine.
3. Backend runtime config: `app_settings` migration, wire `Settings`/`ArcSwap`, dynamic CORS, routines
   read live intervals, `GET/PATCH /api/admin/settings`.
4. Backend registration: `invited_by`, `invites`, modes on `/api/public/register` (standalone).
5. Resolver: SQLx migrations, `backend_state`, heartbeat consumer, operator-token auth + session,
   native admin API, per-instance proxy, config-matrix fan-out, settings, stale-prune + invite-cleanup
   routines.
6. Resolver registration/selection: modes, invites (+ `instance_pin`), strategies, pin-delta, capacity.
7. Frontend: `/admin/resolver`, config matrix, settings panels, invitation graph, env-lock/restart UI.
8. Tests (§14) + doc updates (§15).

## 17. Open questions

- Exact resolver routine intervals (stale-prune vs delegation TTL fraction; invite-cleanup cadence).
- Whether operator-approved backend registration needs an explicit approve step or trusts the shared
  secret + manual `backends` provisioning (today it trusts the secret).
- Config-matrix UX for backends on different code versions (field set drift) — surface unknown fields
  read-only.
