# Feature 24 — Resolver admin dashboard & runtime-config frontend

The frontend half of [feature 23](23_resolver_admin_and_runtime_config.md). Feature 23 shipped the
whole **backend + resolver** control plane (delegation auth, heartbeat, runtime settings, registration
rules, selection strategies, the resolver's native admin API + per-instance proxy + config-matrix).
This feature builds the UI that drives it. It is spec-only here; the frontend is built separately.

Grounding: routing/state/data conventions in [`05_FRONTEND_ARCHITECTURE.md`](../05_FRONTEND_ARCHITECTURE.md);
the endpoints in [`06_API_REFERENCE.md`](../06_API_REFERENCE.md) + [`07_RESOLVER_ARCHITECTURE.md`](../07_RESOLVER_ARCHITECTURE.md).

## 1. Overview & goals

Three surfaces, one shared settings component:

- **`/admin`** (existing, backend user-auth) — unchanged single-instance dashboard, **plus** a new
  Settings tab and Routines tab (this instance's runtime config) and invite management.
- **`/admin/resolver`** (new, **resolver** operator-auth) — the fleet dashboard: aggregate overview,
  per-instance drill-down via the proxy, config-matrix with diff + set-all, registration/selection/
  capacity policy, invite management, the resolver's own settings.
- **User-facing invitation graph** — a small "who invited me / whom I invited" view.

**Preserved invariant:** a standalone backend (no resolver) never shows `/admin/resolver`; its `/admin`
Settings/Invites tabs run against the backend directly. The frontend detects topology the same way it
does today (`/archypix-resolver/info` on the configured global domain).

## 2. Decisions

1. **Resolver auth is a separate identity from user auth.** The operator token is *not* a user login;
   it exchanges for a `ResolverAdminSession` JWT + a refresh token held in a dedicated store
   (`resolverAuth`), never mixed into `auth.ts`. `/admin/resolver` is gated by a resolver-session guard,
   not `ProtectedRoute`'s `adminOnly`.
2. **A dedicated resolver API client.** A second axios instance (`resolverClient`) targets the **global
   domain** (the resolver) with the `ResolverAdminSession` bearer + its own 401→refresh interceptor,
   parallel to `apiClient`. The per-instance proxy is reached through it (`/api/resolver-admin/instances/
   {back_domain}/api/admin/…`), so the fleet dashboard never needs a user token on any backend.
3. **One `SettingsPanel`, metadata-driven.** Both dashboards render the same component from the
   `FieldMeta[]` the API returns (`06 §7 GET …/settings`); field type/enum/lock/secret/restart/group all
   come from metadata, so a new backend setting needs **zero** frontend change. The panel is pointed at
   either `apiClient` (backend `/admin`) or `resolverClient` (resolver settings), or a config-matrix
   adapter (fan-out).
4. **Config-matrix is a diff-first table**, not N independent editors — the whole point is spotting and
   collapsing divergence across the fleet with a single set-all.

## 3. Routing & entry

Add to `src/App.tsx`:

| Path              | Page                | Auth             | Notes                                              |
|-------------------|---------------------|------------------|----------------------------------------------------|
| `/admin/resolver` | `ResolverAdminPage` | resolver session | fleet dashboard; own login screen when no session  |
| `/admin` (tabs)   | `AdminPage`         | admin only       | gains **Settings**, **Routines**, **Invites** tabs |

`ResolverAdminPage` renders its own login form (operator token → session) when `resolverAuth` is empty,
else the tabbed dashboard. A `ResolverProtected` wrapper redirects to that login on a missing/expired
session (after a refresh attempt).

## 4. Resolver auth & client

- **Store `stores/resolverAuth.ts`** — `{ sessionToken, refreshToken, expiresAt } + set/clear`,
  persisted under `archypix_resolver_admin`. Session is short (§07 G, 900 s); a background refresh
  (`POST /api/resolver-admin/refresh`) rotates both tokens before expiry, mirroring the backend token
  refresh the user client already does.
- **`api/resolverAdmin.ts`** — `login`, `refresh`, `overview`, `backends`, `setCapacity`, `getSettings`,
  `patchSetting`, `resetSetting`, `listInvites`, `mintInvite`, `revokeInvite`, `configMatrix`,
  `configMatrixPatch`, and a generic `proxy(backDomain, method, path, body)` for per-instance drill-down.
- **Hooks `hooks/useResolverAdmin.ts`** — TanStack Query wrappers; query keys under
  `queryKeys.resolverAdmin.*` in `lib/constants.ts`.

## 5. Dashboard surface (`/admin/resolver` tabs)

- **Overview** — fleet Σ users/pictures/storage (from stored heartbeat metrics), backend cards with
  health / reachable / last-heartbeat, registration stats. (`GET /overview`.)
- **Backends** — per-backend state rows; each opens a **drill-down** that thin-proxies the backend's
  existing `/api/admin/*` (reusing the *same* admin components the backend `/admin` renders, but pointed
  at `resolverClient.proxy`). An unreachable backend shows why (no heartbeat / delegation expired) and
  disables the drill-down (proxy returns 503).
- **Config matrix** — a field × backend table: per field, the distinct values across reachable backends
  are highlighted when they diverge; a "set all" (or a target subset) fans out one `PATCH`
  (best-effort, per-backend success/failure surfaced inline). Unknown fields on a lagging backend render
  read-only (version/field drift, §23 open-question). (`GET/PATCH /config-matrix`.)
- **Settings** — the resolver's *own* runtime config, via the shared `SettingsPanel` on `resolverClient`
  (selection strategy, `pin_importance`, registration mode, CORS, routine intervals, delegation TTL).
- **Registration & placement** — friendly editors over the same settings (mode dropdown; strategy +
  `pin_importance`; `static_backend`); per-backend **capacity** (`accepting_registrations`, `max_users`)
  via `PATCH …/backends/{d}/capacity`.
- **Invites** — mint (max-uses / expiry / optional `instance_pin` picked from the backend list), list,
  revoke.

## 6. Shared `SettingsPanel` (both dashboards)

Driven entirely by `FieldMeta` (`06 §7`):

- Group fields by `group`; render an input per `kind` (`string`/number/`bool` toggle/enum `variants`
  dropdown; `secret` → masked, write-only; list types → chips).
- **Env-locked** (`locked` / `source == "env"`) → read-only with a "defined by environment" badge; PATCH
  disabled.
- **`restart_required`** → a "takes effect after restart" notice on change.
- **Provenance** chip (`default` / `env` / `db`); a "reset to default" action on `db`-sourced fields
  (`DELETE …/settings/{key}`; hidden/greyed when locked).
- **Routines tab** consumes `GET /api/admin/routines`: live status (last run / in-flight / last error /
  total runs) per routine, its tuning fields inline (same panel), and a **Trigger now** button
  (`POST …/routines/{name}/trigger`).

## 7. Invitation graph (user-facing)

- `GET /me/invitations` → a small view (in `SettingsPage` or a dedicated card): "invited by X" +
  the list of users you invited. Minting/among-users gating follows `registration_mode`
  (`06 §6 invites`); the mint UI is hidden when the mode forbids the caller.

## 8. Edge cases

- **Env-locked field PATCH** — the button is disabled client-side; the API also returns `409`/`400`,
  surfaced as a toast if it ever races.
- **Unreachable backend** — omitted from config-matrix columns (or shown greyed with a reason); its
  proxy drill-down returns `503` and renders an "unreachable" state.
- **Fan-out partial failure** — the matrix shows per-backend ✓/✗ with the error; never atomic.
- **All instances full/closed** on register — the public register flow surfaces the resolver's `503`
  reason (operator-facing text).
- **Field-set drift across versions** — unknown fields from a lagging backend are read-only in the
  matrix; the resolver's own settings panel only knows the resolver's field set.
- **Secret fields** never round-trip a value (metadata omits it); the input is write-only.

## 9. Out of scope / follow-ups

- No per-operator accounts (the operator token *is* the credential — §23 §5.1); multi-operator RBAC is a
  later concern.
- Rich charts/time-series over heartbeat metrics (current view is point-in-time totals).
- Migrating a user between instances from the UI (the `/api/update` mapping exists; no UI yet).

## 10. Work breakdown

1. `stores/resolverAuth.ts` + `resolverClient` (axios + refresh interceptor) + `ResolverProtected`.
2. `api/resolverAdmin.ts` + `hooks/useResolverAdmin.ts` + `queryKeys.resolverAdmin`.
3. `SettingsPanel` (metadata-driven) + Routines tab; wire into backend `/admin` first (simplest client).
4. `ResolverAdminPage` shell + login; Overview + Backends tabs.
5. Per-instance proxy drill-down (reuse backend admin components via `resolverClient.proxy`).
6. Config-matrix table (diff + set-all/targeted fan-out).
7. Registration/placement/capacity editors + Invites CRUD (+ `instance_pin` picker).
8. User-facing invitation graph.

## 11. Doc updates

- `05_FRONTEND_ARCHITECTURE.md` — add the `/admin/resolver` route, the `resolverAuth` store + client, and
  the shared `SettingsPanel` to the route/state/data sections as it ships.
- `99_ROADMAP_MVP.md` — flip the three feature-23 items' "frontend pending" once this lands.
