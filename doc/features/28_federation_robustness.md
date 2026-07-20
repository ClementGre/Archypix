# Federation robustness

Hardening pass over the federation layer: stop a single unreachable peer from breaking read
paths, bound every outbound call, refactor the per-verb REST surface into one typed
message envelope, make the interactive verbs crash-atomic with clear user-facing errors, add
rate limiting + observability, and delete the dead `federation_messages` machinery. Pairs with
`doc/03_BACKEND_ARCHITECTURE.md` §G (federation consistency rules) and `doc/01_GENERAL_SPECIFICATIONS.md` §5.

## 1. Overview & goals

The federation layer works but is brittle in a handful of concrete ways, all traceable to "a peer
is slow, down, or on a different version":

- A **single unreachable remote owner 500s the entire picture list** — `presign_for_picture_list`
  propagates the remote-presign error, so a page containing one down owner's picture shows *nothing*,
  including the caller's own pictures.
- **Outbound federation calls have no timeout** (`HttpClient::new()` in `main.rs`); only the auth
  request is bounded. A hung peer ties up the request handler indefinitely.
- The **token handshake is fragile**: one setting doubles as the per-request HTTP timeout *and* the
  async-grant wait budget (1 s), so first contact routinely times out; the nonce/token are keyed only
  by remote domain with no single-flight, so concurrent first-contacts race and one loses.
- The **transport is per-verb REST** with heavy copy-pasted plumbing and no protocol versioning.
- **Interactive verbs** (accept/reject/edit_request/claim) are not crash-atomic and surface a raw 500
  on a down peer instead of a clear "try later".
- The **`federation_messages` table + its domain types are 100 % dead code**.
- **No rate limiting** on any federation endpoint, and no operator visibility into abuse.

Goals: no read-path outage from a down peer; every outbound call bounded and fast-failing;
one typed, versioned message envelope; crash-atomic interactive verbs with clean errors; backend
rate limiting with an admin observability surface; dead code removed.

Non-goals: a durable outbox / message-tracking table (explicitly rejected — announcements already
converge from state via the pipeline, and the interactive verbs are user-present with a clear
retry); backward-compatible protocol decoders (see §4 — per-message **exact-match** versioning);
resolver-side rate limiting (the resolver has none today and is out of scope).

## 2. Decisions

1. **Read paths degrade, never 500, on a down peer.** A remote-presign failure yields an absent
   thumbnail (client renders its existing placeholder) plus an explicit `owner_reachable = false`
   flag on the list item; the single-picture presign returns `503`, not `500`.
2. **All outbound calls are bounded**, with the connect timeout, the overall request timeout, and the
   grant-wait budget as *three distinct* settings. Federation tokens are **proactively refreshed**
   before expiry and handshakes are **single-flighted** per peer.
3. **One authenticated message endpoint** — `POST /api/federation/message` carrying a tagged-enum
   `FederationMessage` → `FederationResponse`. `auth/request`, `auth/grant`, and `pictures/presign`
   stay on dedicated routes (they bootstrap the token or use a different auth model). This is a
   **breaking wire change**, taken now deliberately while the app is under heavy development.
4. **Per-message-type protocol version, exact match.** Each message type owns a `const VERSION`; the
   receiver rejects any other version with a clear directional error. Exact-match means a passing
   check guarantees both request *and* response are mutually understood — no backward-compat layer.
5. **Interactive verbs are crash-atomic** via deliver-then-commit ordering (the documented
   transaction rule, which `accept` currently violates), return a **typed transient/permanent** error,
   and `claim` is made idempotent.
6. **Delete `federation_messages`**, its enum type, the `domain::federation` dead types, and the dead
   `PictureEditRequest.idempotency_key`.
7. **Backend rate limiting** with configurable per-peer frequency limits + hardcoded structural batch
   ceilings, `warn!` on every rejection, and a Redis-backed recent-rejections store surfaced in a new
   admin "Rate limiting" tab.

---

## 3. Read-path failure isolation

### 3.1 Picture list (`services::pictures::presign_for_picture_list`)

Step 5 (cross-instance groups) currently does `federation.presign_remote_pictures(...).await?` — the
`?` fails the whole list. Change it to isolate per owner-group:

- Wrap each `(owner_username, owner_instance)` group's remote presign in a `match`. On error: `warn!`
  (owner domain + picture count), leave those pictures' URLs **absent**, and record their ids as
  **unreachable**. Continue with the remaining groups. The reachable pictures (owned, same-backend,
  other reachable owners) render normally.
- The absent-thumbnail path already exists (a pending/non-thumbnailable picture is left out so the
  client shows a file-type placeholder — `pictures.rs:1622`); an unreachable remote reuses it.

### 3.2 `PictureListItem.owner_reachable`

Add `owner_reachable: bool` to `PictureListItem` (default `true`). Set `false` for pictures whose
owner-group presign failed in §3.1, so the frontend can render a distinct "owner offline" tile
(greyed, retry affordance) rather than a generic placeholder. Owned / same-backend / reachable-owner
pictures are always `true`. A cross-instance picture with **no active token** stays `true` with an
absent URL (that's a "no thumbnail yet" state, not an outage).

### 3.3 Single-picture presign (`presign_variant_for_picture`)

Used by `GET /pictures/{id}/url`, the lightbox, and public-share view. On a remote-presign failure it
currently returns `500`. Map the remote branch's transient failure to
`AppError::ServiceUnavailable` (`503`) with a clear message ("The owner's instance is unreachable —
try again later"), distinct from `Ok(None)` (no thumbnail exists). The frontend shows a retryable
error rather than a broken image. (Thumbnail-absent still returns `Ok(None)`.)

---

## 4. Outbound HTTP hardening

### 4.1 Timeouts — three distinct clocks

Build the federation `reqwest::Client` (in `main.rs`) with an explicit connect + request timeout
instead of `HttpClient::new()`, and give the async-grant wait its own budget. Replace the single
overloaded `FEDERATION_REQUEST_TIMEOUT_MS` with:

| Setting                         | Role                                                                       | Default |
|---------------------------------|----------------------------------------------------------------------------|---------|
| `FEDERATION_CONNECT_TIMEOUT_MS` | TCP/TLS connect bound — a *down* peer fails fast                           | `2000`  |
| `FEDERATION_REQUEST_TIMEOUT_MS` | **repurposed:** overall per-call HTTP timeout — a *slow* peer is bounded   | `10000` |
| `FEDERATION_GRANT_WAIT_MS`      | how long `get_or_wait_federation_token` polls Redis for the grant callback | `12000` |

The client-level timeouts cover **every** outbound call — including `resolve_backend_url`, which has
none today. The per-call `.timeout(...)` on `ensure_federation_token`'s auth request is removed in
favour of the client default (the grant *wait* uses `FEDERATION_GRANT_WAIT_MS`).

### 4.2 Proactive token refresh

The federation token (`RedisKey::FederationToken`) is refreshed *before* it expires so the request hot
path is never a cold handshake.

- Store the token **with its local expiry**: the cache value becomes a small JSON `{ token,
  expires_at }` (`expires_at` = local epoch seconds, computed from the grant's relative TTL — see
  §4.4). Bump `store_federation_token`, `ensure_federation_token`, and `get_or_wait_federation_token`
  accordingly.
- On a cache read in `ensure_federation_token`:
    - `now < expires_at − FEDERATION_TOKEN_REFRESH_MARGIN_SECS` → return the token, no refresh.
    - within the margin but still valid → return the token **and** spawn a background single-flight
      refresh (§4.3). The caller never blocks.
    - absent / expired → synchronous single-flight handshake (first contact only).

New setting `FEDERATION_TOKEN_REFRESH_MARGIN_SECS` (default `300`).

### 4.3 Single-flight handshake

Guard the handshake per peer so a burst of cold requests to one domain collapses to a single
`auth/request` (today they all fire and race the nonce, which is keyed only by domain, so only one
grant validates). Acquire a Redis lock `RedisKey::FederationRefreshLock(domain)` via `SET NX EX`
(TTL ≈ `FEDERATION_GRANT_WAIT_MS`) before starting a handshake:

- Winner performs `auth/request` and waits for the grant.
- Losers (absent-token case) fall through to the existing `get_or_wait_federation_token` poll loop —
  they wait on the winner's grant landing in the cache.
- Losers (stale-but-valid case) simply keep using the current token.

The nonce continues to be minted+persisted before the request and echoed by the grant (unchanged
poisoning guard); with single-flight there is at most one pending nonce per domain.

### 4.4 Clock-skew-independent grant TTL

`auth/grant` currently carries an **absolute** `expires_at` epoch, so the receiver's computed TTL is
wrong under cross-instance clock skew. Change `FederationAuthGrant` to carry a **relative**
`ttl_secs: i64` (duration from issuance); the receiver computes `expires_at = now + ttl_secs` against
its *own* clock. `auth_request` sends `ttl_secs = FEDERATION_JWT_TTL_SECS`; `auth_grant` stores
`now + ttl_secs`. No dependence on synchronized clocks.

### 4.5 Backend-URL cache bust on failure

On a transient delivery failure to a peer, invalidate `RedisKey::FederationBackend(user, domain)` and
re-resolve **once** before surfacing the error — this handles a user who migrated backends (the
resolver remapped them) without waiting out the 1 h cache. The re-resolve is itself bounded by §4.1.
`resolve_backend_url` additionally **serves a stale cached URL if the resolver itself is unreachable**
(connection error, not a 404) so a resolver blip is non-fatal for already-known peers. (The existing
404 → "domain is its own backend" fallback is unchanged but, per feature-27-era decisions, is **not
cached** — a transient resolver 404 must not pin a wrong backend for an hour.)

---

## 5. Message transport: one typed, versioned envelope

### 5.1 The envelope

Replace the eight authenticated verb routes with one:

```
POST /api/federation/message      (AuthFederation)   FederationMessage → FederationResponse
```

`FederationMessage` is an internally-tagged enum; the wire envelope also carries the per-message
version:

```rust
// clients/federation/models.rs
struct FederationEnvelope {
    msg_version: u16,             // checked against the variant's VERSION (§5.3)
    #[serde(flatten)]
    message: FederationMessage,   // #[serde(tag = "type", rename_all = "snake_case")]
}

enum FederationMessage {
    ShareAnnounce(ShareAnnouncementRequest),
    ShareAccept(ShareAcceptRequest),
    ShareReject(ShareRejectRequest),
    ShareRevoke(ShareRevokeRequest),
    PublicShareClaim(PublicShareClaimRequest),
    PicturesAnnounce(PicturesAnnouncementRequest),
    PicturesUnannounce(PicturesUnannouncementRequest),
    PictureEditRequest(PictureEditRequest),
}

enum FederationResponse {  // #[serde(tag = "type")]
    Ack,                                     // revoke / reject / unannounce
    ShareAnnounce { auto_accepted: bool },
    PicturesAnnounce { registered: usize },
    PublicShareClaim(PublicShareClaimResponse),
    PictureEdit { accepted: bool },
}
```

The per-verb request/response structs (`ShareAnnouncementRequest`, …) are **kept** — the envelope
wraps them, it does not flatten their fields away.

### 5.2 Client: one generic `send`

Collapse the ~8 near-identical client methods (each: get token → resolve URL → post → bearer_auth →
trace headers → send → `error_for_status` → map errors) into one:

```rust
trait FederationMessageType {
    const VERSION: u16;
    type Response: DeserializeOwned;
    fn into_message(self) -> FederationMessage;
}

impl FederationClient {
    async fn send<M: FederationMessageType>(
        &self, sender_username: &str, peer_username: &str, peer_global_domain: &str, msg: M,
    ) -> Result<M::Response, AppError> { /* token, resolve, post envelope, classify errors */ }
}
```

`presign_remote_pictures` stays a separate method (unauthenticated, different endpoint). Error
classification (§6.2) and the backend-URL bust-on-failure (§4.5) live once, here.

### 5.3 Handler: one dispatch

`api/federation/handlers.rs::message` takes `AuthFederation` + `FederationEnvelope`, then:

1. **Version check** (§5.4) against the matched variant's `VERSION`.
2. `maybe_set_remote_parent` (trace linkage) — once, here.
3. `match` on the variant → the existing `services::federation::receive_*` function.

Per-message peer binding stays in the arm (e.g. `PictureEditRequest`/`PublicShareClaim` re-check
`requester_instance == auth.claims.sub`). The envelope proves *which* peer is calling; the arm proves
*what* it may assert.

### 5.4 Protocol version — per-message, exact match

Each message type declares `const VERSION: u16`, starting at `1`, co-located with its struct. On
receipt, after tag dispatch, the handler checks `envelope.msg_version == VARIANT::VERSION`; on
mismatch it returns a dedicated `AppError` mapping to **`426 Upgrade Required`**, with a body
`{ error: "version_mismatch", message_type, receiver_version }`.

The client's `send` detects a `426`, reads `receiver_version`, and compares to its own `VERSION` to
produce a **directional** error:

- `receiver_version < ours` → *"The recipient's instance is running an older, incompatible version of
  Archypix."*
- `receiver_version > ours` → *"Your instance is out of date — update to share with this recipient."*

Because the match is exact, a successful check guarantees both sides run the same version *for that
message*, so the response is decodable too — there is deliberately **no** backward-compat decoding.
Bumping a message's shape = bump its `VERSION`. (A single coarse transport/envelope epoch is
explicitly deferred; per-message versioning covers all churn for now.) `pictures/presign` and the
auth handshake are exempt (stable, pre-token contracts).

---

## 6. Interactive-verb delivery (accept / reject / edit_request / claim)

No tracking table. The share state machine + the pipeline's existing announce reconcile remain the
durability substrate; these verbs are one-shot events made crash-safe by ordering.

### 6.1 Deliver-then-commit atomicity

`services::shares::lifecycle::accept_incoming_share` currently commits `IncomingShare = Active`
**before** notifying the sender, then reverts on error (`lifecycle.rs:454`) — a crash between the
commit and the revert leaves the share silently stuck `Active` with the sender never notified. Fix by
restoring the documented rule (03 §G): perform the mutation + the federation delivery in **one
transaction, deliver inside it, commit last**:

```
BEGIN
  set IncomingShare = Active
  send ShareAccept to sender           -- inside the tx
COMMIT                                  -- only if delivery succeeded
```

A crash or delivery failure before `COMMIT` leaves the share `Pending` (clean, re-acceptable); the
manual revert is deleted. The receiver-visibility race (the sender's async picture-announce arriving
before Bob's commit) is absorbed by the announce path's existing `errored → backoff → retry`, which
self-heals — no new machinery. **reject** (`reject_incoming_share`) gets the identical treatment.

### 6.2 Typed transient vs permanent errors

The `send` client (§5.2) classifies the outcome instead of collapsing everything to
`InternalServerError`:

- connect/timeout/peer-`5xx` → **transient** → `AppError::ServiceUnavailable` (`503`) → frontend
  *"The recipient's instance is unreachable right now — try again later."*
- peer-`4xx` (share gone, grant revoked, `404`) → **permanent** → surface the specific reason (no
  "try later" — retrying won't help).
- `426` → the version message (§5.4).

Errored **shares** remain visible to the user as today (the `OutgoingShare` `errored` status +
`last_error_at`); no additional per-message history is stored.

### 6.3 `claim` idempotency

`services::federation::receive_public_claim` mints a fresh derived `OutgoingShare` on every call, so a
visitor retrying after a lost response gets a **duplicate**. Before minting, look up an existing
non-terminal derived share for `(derived_from_public_share_id, requester_username, requester_instance)`
and return it if present. `edit_request` is already last-write-wins idempotent; `accept`/`reject` are
idempotent no-ops on the far side (`Active`/`Tombstoned` short-circuits) — only `claim` needs the
guard.

### 6.4 Same-instance short-circuit unchanged

Same-backend paths (`find_local_user_id` hit) stay direct in-process service calls — no HTTP, no
envelope, no token, no "peer down" class. Untouched.

---

## 7. Stale-announcement guard

Announcements carry the owner's **full current state** (not a delta), applied last-write-wins. Once
retries + timeouts exist (§4/§6), out-of-order delivery becomes likely and a retried *older*
announcement can overwrite newer data. Guard it with the owner's monotonic `updated_at`:

- Add `owner_updated_at: Option<NaiveDateTime>` to `AnnouncedPicture` (`from_picture`: owned → the
  row's `updated_at`; relayed → the stored owner value).
- Add column `pictures.remote_updated_at TIMESTAMP NULL` (received rows), recording the last-applied
  owner `updated_at`.
- `register_received_pictures` (and the metadata-refresh path) **skips** an announced picture whose
  `owner_updated_at <= remote_updated_at`, and otherwise stamps the new value in the same statement.
  A `NULL` incoming value (peer predating the field) is always applied (no regression for old peers).

---

## 8. Delete `federation_messages`

A migration (`cargo sqlx migrate add -r`) drops:

- table `federation_messages`, enum types `federation_message_type` / `federation_direction` /
  `federation_status`, and their indexes / FKs.
- `back/src/domain/federation.rs`'s `FederationMessage*`, `FederationDirection`, `FederationStatus`
  types (keep only `BackendMapping` if still referenced — verify; otherwise drop it too and the
  `domain::federation` module).
- `PictureEditRequest.idempotency_key` (wire field, set at `services::pictures.rs:1279`, never read by
  the receiver). Its apply is already last-write-wins idempotent.

Regenerate `back/migrations/schema.sql` and the `.sqlx` cache.

---

## 9. Rate limiting & observability

Backend-only. Reuses `infra::ratelimit::check` (fixed-window Redis, fail-open). Structural **batch
ceilings are hardcoded consts** (must never block a legitimately-large page from a differently-
configured peer); request **frequency limits are configurable**.

### 9.1 Caps & limits

- **Presign** (`/pictures/presign`, unauthenticated, token-gated): hardcoded
  `MAX_PRESIGN_BATCH = 10_000` on `pictures.len()` (`400` if exceeded); batch the N+1 lookups
  (`find_picture_by_token` → `find_by_id`) into set-based queries; a very generous per-source-IP
  window (`FEDERATION_PRESIGN_RATE_MAX` / `_WINDOW_SECS`) — per-IP ≈ per-peer-backend here, so it must
  not throttle a busy legitimate peer.
- **Authenticated verbs** (`/message`): a per-peer-domain fixed-window limit
  (`bucket = "federation:{peer_global_domain}"`, `FEDERATION_RATE_MAX` / `FEDERATION_RATE_WINDOW_SECS`)
  applied in the `message` handler after auth; hardcoded `MAX_ANNOUNCE_BATCH = 10_000` on inbound
  `pictures` / `picture_ids` length.

All frequency limits are large-by-default, documented as "never trips normal behaviour", and
DB-editable under `group::RATE_LIMITS` (so they render in the metadata-driven `SettingsPanel`). Every
rejection already `warn!`s in `ratelimit::check`.

### 9.2 Recent-rejections store

Record rejections in Redis (where the counters already live — cross-replica, restart-surviving, TTL'd)
**aggregated per minute** so an attack's flood stays bounded:

- `RedisKey::RateLimitEvent(category, minute_epoch)` → an incremented counter, TTL
  `RATE_LIMIT_EVENT_RETENTION_SECS` (default `86400`). `category` ∈ {`login`, `register`,
  `public_upload`, `federation`, `presign`}.
- `ratelimit::check` (or a thin wrapper carrying the category) increments the current-minute bucket on
  every rejection.

### 9.3 Admin surface

- `GET /api/admin/rate-limits` → the recent-rejection buckets (per category, per-minute counts over
  the retention window) + a simple `attack_suspected` flag when a category exceeds a threshold in the
  last window.
- New **admin dashboard "Rate limiting" tab**: (1) the `RATE_LIMITS` settings section (existing
  `SettingsPanel`), (2) a recent-rejections timeline per category with the attack flag — enough to
  answer "is an attack on?" and "is a limit blocking legitimate traffic?" without a full logs table.
- Reachable through the resolver's existing delegation proxy, so a fleet operator can open any
  instance's tab. **No resolver-side rate limiting or tab** (out of scope).

---

## 10. Presign cache TTL + frontend auto-refresh

- **Backend:** the remote presign response carries the URL's **expiry**; the recipient caches the
  cross-instance URL under `min(local S3_PRESIGN_TTL − margin, remote expiry)` so the advertised
  lifetime is truthful (today it's cached under the *local* TTL, which can outlive the owner's actual
  presign). Extend `PresignResultItem` with `expires_at` and thread it into the cache set in
  `presign_for_picture_list` / `presign_variant_for_picture`.
- **Frontend:** treat an expired / `403` image URL as "refresh" — re-request a fresh presigned URL
  (re-fetch the list item, or call `GET /pictures/{id}/url`) and swap it in rather than showing a
  broken image. This also fixes the observed **stale-thumbnail-on-session-resume** (a URL that expired
  while the tab was backgrounded now self-recovers without a manual reload).

---

## 11. Configuration (new / changed settings)

All under `group::FEDERATION` unless noted; DB-editable (runtime) unless stated.

| Key                                    | Default | Note                                                                 |
|----------------------------------------|---------|----------------------------------------------------------------------|
| `FEDERATION_CONNECT_TIMEOUT_MS`        | `2000`  | new; connect bound (client-level, restart to rebuild client)         |
| `FEDERATION_REQUEST_TIMEOUT_MS`        | `10000` | **repurposed** to overall per-call timeout (was 1000, overloaded)    |
| `FEDERATION_GRANT_WAIT_MS`             | `12000` | new; async-grant poll budget                                         |
| `FEDERATION_TOKEN_REFRESH_MARGIN_SECS` | `300`   | new; proactive-refresh lead time                                     |
| `FEDERATION_RATE_MAX`                  | `6000`  | new (`group::RATE_LIMITS`); per-peer authenticated-verb max / window |
| `FEDERATION_RATE_WINDOW_SECS`          | `60`    | new (`group::RATE_LIMITS`)                                           |
| `FEDERATION_PRESIGN_RATE_MAX`          | `12000` | new (`group::RATE_LIMITS`); per-IP presign max / window              |
| `FEDERATION_PRESIGN_RATE_WINDOW_SECS`  | `60`    | new (`group::RATE_LIMITS`)                                           |
| `RATE_LIMIT_EVENT_RETENTION_SECS`      | `86400` | new (`group::RATE_LIMITS`); recent-rejection retention               |

Hardcoded consts (not settings): `MAX_PRESIGN_BATCH = 10_000`, `MAX_ANNOUNCE_BATCH = 10_000`.

The connect timeout and the reqwest client are rebuilt at startup, so `FEDERATION_CONNECT_TIMEOUT_MS`

+ `FEDERATION_REQUEST_TIMEOUT_MS` are `restart_required()`; the grant-wait / refresh-margin / rate
  limits are read live.

---

## 12. Edge cases

- **Peer down mid-list** → its pictures render as "owner offline" tiles; the rest of the page is
  unaffected (§3).
- **First contact under load** → single-flight collapses to one handshake; the grant-wait budget gives
  the callback room (§4.2–4.3).
- **User migrated backends** → first delivery fails, cache is busted + re-resolved once, retry lands on
  the new backend (§4.5).
- **Version skew** → `426` with a directional message; no partial/garbled apply (§5.4).
- **Retried `claim`** → returns the existing derived share, no duplicate (§6.3).
- **Out-of-order announce** → the older one is dropped by the `remote_updated_at` guard (§7).
- **Old peer** (predates a field) → `#[serde(default)]` on new envelope/announce fields keeps it
  working for unchanged message versions; a `NULL owner_updated_at` is always applied.
- **Redis down** → rate limiter fails open (unchanged), presign/token caches miss-through (bounded by
  timeouts), recent-rejection recording is best-effort.

## 13. Frontend

- **Gallery:** render an "owner offline" tile for `owner_reachable === false` (distinct from the
  file-type placeholder); optional inline retry.
- **Presigned URLs:** auto-refresh on expiry / `403` (§10).
- **Share actions:** map `503` → "recipient instance unreachable, try later"; `426` → the version
  message; `4xx` → the specific reason.
- **Admin "Rate limiting" tab:** settings section + recent-rejections timeline + attack flag (§9.3).

## 14. Testing

- List with a picture whose owner backend is unreachable → `200`, the picture flagged
  `owner_reachable=false`, others present (§3).
- Outbound call against a black-holed peer returns within the timeout, not hanging (§4.1).
- Concurrent cold first-contacts → exactly one `auth/request` (single-flight) (§4.3).
- Grant with a relative TTL under simulated clock skew → correct local expiry (§4.4).
- `message` endpoint round-trips each variant; a wrong `msg_version` → `426` with `receiver_version`
  (§5).
- `accept` interrupted before commit leaves the share `Pending` and re-acceptable; a down sender →
  `503` (§6.1–6.2).
- Duplicate `claim` → one derived share (§6.3).
- Stale (older `owner_updated_at`) announce is ignored (§7).
- Presign batch over `MAX_PRESIGN_BATCH` → `400`; per-peer verb flood → `429` + a recorded rejection
  bucket (§9).
- Migration up/down drops+restores `federation_messages` cleanly; `SQLX_OFFLINE` check passes (§8).

## 15. Documentation updates

- `doc/06_API_REFERENCE.md` §8 — replace the eight federation verb routes with `POST
  /api/federation/message` (envelope + variants + `426` semantics); keep `auth/request` (now
  `ttl_secs`), `auth/grant`, `pictures/presign` (now with `expires_at`); add `GET
  /api/admin/rate-limits`; note `owner_reachable` on the list item.
- `doc/03_BACKEND_ARCHITECTURE.md` — §C module notes (client `send`, `federation` message handler),
  §G add the deliver-then-commit note for accept/reject and the message-envelope/version rule; drop
  the `federation_messages` mention if any.
- `doc/07_RESOLVER_ARCHITECTURE.md` — note the backend serves stale resolve results to peers on
  resolver failure (§4.5) — resolver behaviour unchanged.
- `doc/99_ROADMAP_MVP.md` — tick **Federation robustness** and record deviations.

## 16. Work breakdown

1. Migration: drop `federation_messages` + enums; add `pictures.remote_updated_at`; regenerate
   `schema.sql` + `.sqlx`.
2. Timeouts + client rebuild (§4.1); relative grant TTL (§4.4); backend-URL bust + serve-stale (§4.5).
3. Token store-with-expiry + proactive refresh + single-flight (§4.2–4.3).
4. Message envelope + `FederationMessage`/`FederationResponse` + per-message `VERSION` + client
   `send` + handler dispatch; drop the old routes/methods; `426` error + directional client mapping
   (§5); remove dead `idempotency_key` (§8).
5. Read-path isolation + `owner_reachable` + single-picture `503` (§3).
6. Deliver-then-commit for accept/reject; transient/permanent classification; `claim` idempotency
   (§6).
7. Stale-announce guard end-to-end (§7).
8. Rate limits (caps + per-peer/-IP windows) + Redis recent-rejections + `GET /api/admin/rate-limits`
   (§9).
9. Presign expiry in the response + `min()` caching (§10 backend).
10. Frontend: owner-offline tile, presign auto-refresh, error toasts, admin "Rate limiting" tab
    (§13).
11. Tests (§14); doc updates (§15); roadmap tick.
