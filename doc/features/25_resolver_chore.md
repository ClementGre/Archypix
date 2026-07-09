- Show fleet dashboard access buttons only if the instance of the user uses a resolver (see below the new resolver endpoint that will allow this)
- Connect to the resolver of the user’s global domain if authenticated (the frontend allow users to connect to a custom domain name), otherwise
  connect to the instance default global domain.
- When registering, allow to edit the global domain even if registration is closed, that way if typing a global domain that has closed registration,
  the user can still change it without going back to the login page.
- When connecting to the resolver fleet dashboard, allow to specify a different resolver domain than the one of the authenticated user, fallbacked to
  the instance
  default global domain (env configured) (a bit the same way we can connect to the public app using any backend domain, but same for the resolver
  fleet dashboard, with the same cors warning. BTW: Replace that cors warning by a real ping of the server to check if it’s alive, and to check if its
  cors is ok, that way we can show a real error message to the user if the cors is invalid, and show nothing if the cors is valid).
- resolver prefix: drop `.well-known/webfinger` entirely — it's not RFC-compliant here anyway, and reusing that exact path risks colliding with
  real WebFinger/Matrix discovery if a self-hoster also runs Mastodon/Matrix on the same apex domain. Instead, the **resolver's entire router**
  (its existing `/api/public/*`, `/api/resolver-admin/*`, `/api/backends/*`, `/health`, plus the new `/info` and `/resolve` below) is nested,
  unchanged internally, under one top-level prefix: `/archypix-resolver/`. That's a router mount-point change only — no individual handler paths
  move. A self-hoster running their own resolver then has exactly one forwarding rule to configure and can keep the rest of the domain for anything
  else; no `.well-known` special-casing, no conflict with other apps on the domain.
  - The **backend** is untouched by this — it keeps serving its own `/api/public/register` etc. unprefixed, as today (see
    `doc/02_INFRASTRUCTURE_DESIGN.md` §Resolver and `doc/07_RESOLVER_ARCHITECTURE.md` §G) — except for one new foreign route it also answers,
    `GET /archypix-resolver/info` (see below). The backend never had a "keep my domain clean" problem (it already owns its whole (sub)domain), so
    it gets no other changes and no route duplication.
  - `/archypix-resolver/info` and `/archypix-resolver/resolve` are the two exceptions to the "everything is `api_url`-relative" rule below: both
    are **fixed, directly-callable paths** at whatever domain is being targeted, never routed through `api_url`. Federation resolution
    (`@user:domain` → backend, today's single-shot `.well-known/webfinger` call, see `doc/01_GENERAL_SPECIFICATIONS.md` §5.2) is a hot path — it
    must stay one HTTP call, not a bootstrap-then-resolve pair. Only the non-hot-path, UI-triggered surface (registration, dashboard) goes through
    the `api_url` indirection, since one extra round trip there is imperceptible.
- no-resolver / different-host bootstrap: add `GET /archypix-resolver/info`, always answered directly (never `api_url`-relative — see above) by
  whatever sits at the target domain, same shape either way:
  ```json
  { "is_resolver": false, "api_url": "https://archypix.example.com" }
  ```
  or, when a resolver exists:
  ```json
  { "is_resolver": true, "api_url": "https://example.com/archypix-resolver" }
  ```
  `api_url` is used **only** for the heavier surface: `${api_url}/api/public/register` etc. always; `${api_url}/api/resolver-admin/...` when
  `is_resolver`. Resolution itself never waits on this call — `GET /archypix-resolver/resolve?user={user}&domain={domain}` is hit directly at the
  target domain in one request (replaces the old `resource=archypix:@user` webfinger query), exactly like today's single-hop webfinger lookup, and
  is simply absent/404 when there's no resolver (a standalone backend has nothing to resolve — its own domain already is the answer). This still
  subsumes "resolver served on a different domain" for the *registration/dashboard* surface (a resolver operator can point `api_url` elsewhere),
  but `info` and `resolve` themselves must be answered directly at the domain being queried — if a resolver truly lives on a separate host, that
  domain still needs to forward exactly these two routes (not the rest) so federation resolution stays single-hop. This shrinks the self-hoster's
  manual step from "reverse-proxy the whole `/api` surface" to "serve one static JSON file plus forward two fixed routes."

---

## Implementation status — DONE

- **Resolver** (`resolver/`): whole router nested under `/archypix-resolver/` (`api.rs`); `.well-known/webfinger`
  handler dropped; new `api/bootstrap.rs` = `GET /info` (`{ is_resolver: true, api_url }`) + `GET /resolve` (replaces
  webfinger, returns `{ backend_url }`, 404 on unknown/mismatch). Config gained `USE_HTTPS` + nullable `PUBLIC_URL`
  (core) with a `public_url(&s)` derive (`{scheme}://{GLOBAL_DOMAIN}/archypix-resolver` default). `.env.example` updated.
- **Backend** (`back/`): `api/bootstrap.rs` = `GET /archypix-resolver/info` (`is_resolver:false`, own public URL) **and**
  `GET /archypix-resolver/resolve` (confirms the user exists, returns this backend's own public URL — same shape as the
  resolver), both CORS-open and always served. Serving `resolve` on the backend lets a single-domain deployment whose
  backend domain differs from the global domain and runs no resolver forward `/archypix-resolver/` from the global domain
  to the backend and still resolve in one hop (refines the original spec's "standalone has nothing to resolve").
  `/.well-known/webfinger` route + handler removed. `FederationClient::resolve_backend_url` (renamed
  `webfinger.rs`→`resolve.rs`) now hits `/archypix-resolver/resolve` in one call; a 404 (no `resolve` endpoint at all)
  falls back to the domain itself. `ResolverClient` internal calls prepend the `/archypix-resolver` prefix.
- **Frontend** (`front/`): `api/resolve.ts` (`getResolverInfo` + `resolveConnection`, replaces `webfinger.ts`); auth store
  holds `isResolver`/`resolverUrl`; `login`/`register` bootstrap `/info`. **Every direct-to-domain public call bootstraps
  `/info` first and hits `${api_url}/api/public/…`** (never a bare `originFor(domain)/api/public/…`): `register`,
  `previewInvite`, `getRegistrationInfo` — so they land under the `/archypix-resolver` prefix in resolver mode. `TopBar`
  Fleet-dashboard entry gated on `isResolver`; `RegisterPage` keeps the instance editable when registration is closed;
  `InstanceCorsWarning` → `InstanceHealthWarning` (live reachability + CORS ping); `resolverAdmin.ts`/`resolverAuth`/
  `ResolverLogin` target a chosen resolver domain (dynamic axios `baseURL` = the resolver `api_url`; rejects when unset so a
  query never falls back to the app origin), defaulting to the user's instance. The backend `/admin` **Invites** tab shows a
  "managed on the resolver" notice (links to the fleet dashboard + Profile) instead of an empty/misleading local list when
  `isResolver`.

**Not run here:** the Rust integration suites (`back`, `resolver`) — the project's Postgres isn't available in this
environment. All four crates + the frontend compile clean; the one backend logic path changed (federation resolve) is
bypassed in tests via cache pre-seeding.
