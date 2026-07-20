# Backend Security Audit

Scope: the Rust backend (`back/`), focused on authentication, authorization, and the
checks behind the endpoints called from `front/src/api`. The goal is to confirm there is
no obvious critical issue (missing auth, a user mutating another user's data), and to list
the systemic concerns (DoS / spam vectors) worth hardening before/after MVP.

**Overall assessment:** the core authorization model is sound. Every authenticated and admin
endpoint is gated by the correct extractor, and every per-resource operation is scoped to the
caller's `user_id` / `owner_id` / `recipient_id` in the SQL itself. No missing-auth route and
no cross-user IDOR was found. The findings below are mostly hardening items (rate limiting,
quotas, spam caps) plus a few low-severity validation/robustness gaps.

---

## 1. What was verified as SOUND

### 1.1 Auth extraction is consistent and fails closed

- `AuthUser` / `AuthAdmin` / `AuthWorker` / `AuthFederation` / `AuthResolver` extractors all
  decode the JWT, check `token_type`, and reject otherwise (`back/src/api/middleware/*`).
- Auth is **extractor-based**, not a router layer, so each handler must declare it. This was
  audited exhaustively:
    - All 18 admin handlers take `AuthAdmin` (`api/admin/handlers.rs`).
    - All authenticated user handlers (pictures, jobs, tags, tagging-services, hierarchies,
      settings, shares, profile) take `AuthUser`.
    - The only un-gated user handlers are `register` and `get_public` (intentionally public),
      and the only tokenless federation handlers are `auth_request`, `auth_grant`, and
      `presign_pictures` (auth is the per-picture token).
- `AuthAdmin` is derived from `AuthUser` + `is_admin` claim — no separate admin token, matching
  the documented model.

### 1.2 JWT handling

- Fixed `Algorithm::HS256` on both encode and decode → no algorithm-confusion / `alg:none`
  downgrade (`infra/crypto.rs`).
- `decode` validates audience (`BACK_DOMAIN`), issuer, and expiry (jsonwebtoken default).
- Worker tokens use a **separate secret** (`worker_jwt`, `WORKER_JWT_SECRET`) — a leaked user
  secret does not forge worker tokens and vice-versa. `decode_any_issuer` only skips the issuer
  check for workers (which may run on any host); audience is still enforced.

### 1.3 Per-resource ownership checks (no cross-user IDOR)

Spot-checked every mutating path; all scope to the caller:

- **Pictures**: `get_picture_details`, `presign_picture_variant` reject `pic.local_user_id != user_id`
  with `NotFound`. EXIF edit (`edit_pictures_exif`) validates the whole batch for
  `local_user_id == user_id` **and** `is_owned()` before any mutation (received pictures cannot
  be EXIF-edited).
- **Tags**: `TagRepository::batch_assign` / `batch_remove` constrain writes with
  `picture_id IN (SELECT id FROM pictures WHERE local_user_id = $user)` directly in SQL, so a
  forged `picture_ids` list cannot touch another user's pictures. Only `source = 'manual'` rows
  are mutated — pipeline/share tags are untouchable by user calls.
- **Jobs**: `get_job` / `list_picture_jobs` reject jobs/pictures not owned by the caller.
- **Shares**: `revoke_outgoing_share` checks `share.owner_id == owner_id`; `accept`/`reject`
  check `incoming.recipient_id == acceptor/rejector_id`. All return `NotFound` on mismatch.
- **Tagging services / hierarchies**: repository calls are `get_by_owner_and_id`,
  `delete(owner_id, …)`, `load_owned(user_id, …)` — owner-scoped at the query level.
- **WebDAV / VFS**: every `VirtualFs` operation (list/read/put/delete/move/copy/retag) runs
  through `self.user_id` (the resolved session user); picture resolution comes from
  `PictureRepository::list(self.user_id, …)`. A token holder can only act on their own pictures.

### 1.4 Federation trust boundary

- Inbound handlers bind every action to the **authenticated instance** (`claims.sub` of the
  pairwise federation JWT), not to attacker-supplied fields:
    - `receive_share_announcement` / `receive_pictures_announcement` reject
      `sender_instance != authenticated_instance`.
    - `receive_share_accept` / `receive_share_reject` reject when the share's
      `recipient_instance != authenticated_instance`.
    - `receive_share_revoke` / `receive_pictures_unannouncement` look the share up keyed by
      `(outgoing_share_id, authenticated_instance)`.
- The pairwise-JWT handshake is safe against instance impersonation: a token requested for
  `requester_instance = X` is **delivered out-of-band** to X's backend (resolved via the resolver),
  so a caller cannot obtain a token minted for an instance it does not control.
- `presign_by_picture_tokens` treats the per-picture token as the capability, then confirms the
  resolved picture `is_owned()` by this backend. Tokens are random UUIDv4 (unguessable,
  non-enumerable).

### 1.5 Credential storage

- Passwords: Argon2 with per-hash random salt (`hash_password`).
- Refresh tokens: 256-bit random, stored **hashed** (SHA-256), rotated on every refresh
  (old revoked, new issued), revocable on logout.
- WebDAV tokens: 256-bit random, stored **encrypted** (AES-256-GCM, domain-separated sub-key of
  `JWT_SECRET`), compared in **constant time**, cache keyed by hash. Auth fails closed and skips
  disabled hierarchies.

---

## 2. Findings (hardening / lower-severity)

No finding below is a critical missing-auth or cross-user-write bug; they are availability,
abuse, and validation-hardening items.

> **Resolution status (this PR):** §2.1, §2.2, §2.3, §2.4, §2.5, §2.7, §2.9, §2.10 are **fixed**.
> §2.6 (storage quotas) is deferred to the roadmap. §2.8 is intended behaviour (informational).
> Each fixed item is annotated inline below.

### 2.1 — MEDIUM — [FIXED] Unauthenticated federation `auth/grant` enables token-cache poisoning

`POST /api/federation/auth/grant` (`api/federation/handlers.rs::auth_grant`) is **tokenless**
and writes the supplied `token` into Redis under `federation:token:{issuer_instance}` with no
verification that a matching `auth_request` is pending and **no `nonce` correlation** (the nonce
is carried through the handshake but never checked here).

Impact: any unauthenticated party who can reach the backend can overwrite the cached outbound
federation token for an arbitrary global domain, causing this backend's subsequent federation
calls to that domain to be rejected → **denial of outbound federation** (shares/announcements to
that instance fail) until the cache entry expires. It does not grant inbound access (inbound
still verifies a properly-signed JWT).

Recommendation: correlate the grant to a pending request — store the `auth_request` nonce
(per target domain) when the request is sent, and in `auth_grant` reject grants whose
`issuer_instance`/`nonce` do not match a pending outbound request. Optionally validate that the
grant's `iss`/audience are well-formed before caching.

**Fixed:** `ensure_federation_token` now persists the request `nonce` under
`FederationAuthNonce(domain)` before sending; `store_federation_token` rejects any grant whose nonce
does not match a pending request and consumes the nonce on success (one-time use).

### 2.2 — MEDIUM — [FIXED] No rate limiting anywhere (login brute-force, registration/share spam)

There is no throttling layer (`grep` finds no governor/rate-limit dependency, and no per-IP or
per-user limiter). Consequences:

- `POST /api/auth/login` can be brute-forced; combined with **user-enumeration timing** —
  `login` returns immediately when the username is unknown and only runs Argon2 when it exists,
  so response time distinguishes valid usernames.
- `POST /api/public/register` (standalone mode) registration is unthrottled.
- Federation/resolver lookups and outbound handshakes can be triggered repeatedly.

Recommendation: add a rate-limit middleware (e.g. `tower_governor`) on `auth/*`, `public/*`,
and share/federation entry points. For enumeration, perform a constant-time dummy Argon2 verify
when the user is not found so login latency is uniform.

**Fixed:** added a Redis fixed-window limiter (`infra::ratelimit`, `Cache::incr_ex`). Login is
throttled per username (`RATE_LIMIT_LOGIN_*`) and registration per source IP
(`RATE_LIMIT_REGISTER_*`), both returning `429`. Login now always runs exactly one Argon2 verify —
a dummy one (`verify_password_dummy`) when the user is absent — closing the enumeration timing
side-channel. The limiter fails open if Redis is down.

### 2.3 — MEDIUM — [FIXED] No cap on pending shares (outgoing or incoming) → share spam

`create_outgoing_share` (`services/shares/lifecycle.rs`) enforces no limit on the number of
`pending` outgoing shares a user may create, and the inbound `receive_share_announcement` path
imposes no cap on `pending` incoming shares a recipient can accumulate. A single user (or a
malicious federated instance) can flood a recipient's incoming-share list, or create unbounded
outgoing shares.

Recommendation (as the request suggested): cap the number of `pending` outgoing shares per
sender user, and the number of `pending` incoming shares per recipient (per sender instance),
returning `429`/`409` past the threshold.

**Fixed:** `create_outgoing_share` rejects with `429` once the owner holds
`MAX_PENDING_OUTGOING_SHARES` pending shares; `receive_share_announcement` rejects once the
recipient holds `MAX_PENDING_INCOMING_SHARES` pending incoming shares.

### 2.4 — MEDIUM — [FIXED] Outbound-request amplification / SSRF-flavoured vector via `recipient_instance`

`create_outgoing_share` accepts an arbitrary `recipient_instance` and immediately drives a
resolver resolution + federation HTTP POST to whatever backend that domain resolves to (inside
the request transaction). An authenticated user can therefore make the backend issue outbound
HTTP requests to arbitrary attacker-chosen domains (the federation handshake also `POST`s to the
resolved backend URL). This is bounded by resolvability but is still a request-
amplification / blind-SSRF surface.

Recommendation: validate `recipient_instance` against an allowlist or a format/again-resolvable
check, apply a short timeout (already partly present), and consider not allowing outbound
federation to private/loopback address ranges after resolution.

**Fixed:** `create_outgoing_share` validates `recipient_instance` with
`domain::validation::validate_federation_domain` before any resolver/federation call — rejecting
schemes, ports, paths, whitespace, `localhost`/`.local` domains, and IPv4/IPv6 literals (e.g.
`169.254.169.254`, `127.0.0.1`, `::1`). Internal backend URLs still come only from the trusted
resolver, not this field. Full DNS-rebinding defence (re-checking the resolved IP) remains future work.

### 2.5 — LOW — [FIXED] `initial_tags` on upload-complete bypass tag validation

`complete_upload` (`services/pictures.rs:279`) passes `meta.initial_tags` **straight** to
`TagRepository::batch_assign` without `TagPath::parse(.., allow_protected = false)`, unlike
`PATCH /api/authenticated/tags` (`api/user/tags.rs`) which validates. Consequences:

- A user can self-assign **`SharedToMe.*` manual tags** on their own pictures, contradicting the
  documented contract ("`initial_tags` paths must not start with `SharedToMe`"). Impact is
  limited — the tag is `source = manual`, so it grants no presign access to anyone else's data —
  but it pollutes the reserved namespace and may confuse hierarchy/UI resolution.
- Malformed (non-`[A-Za-z0-9_]`) labels reach the `::ltree` cast and surface as a `500` instead
  of a clean `400`.

Recommendation: run `initial_tags` through the same `TagPath::parse(s, false)` validation used
by the tags endpoint before assigning.

**Fixed:** `complete_upload` now parses every `initial_tags` entry with `TagPath::parse(_, false)`
*before* touching S3, returning `400` on a malformed path or the reserved `SharedToMe` prefix.

### 2.6 — LOW — [DEFERRED] No per-user storage quota; WebDAV body limit disabled

`api/webdav.rs` sets `DefaultBodyLimit::disable()` (needed for large uploads), and there is no
per-user storage quota on uploads (direct API or WebDAV). An authenticated user can exhaust
storage. Bounded to authenticated users, but there is no ceiling.

Recommendation: enforce a per-user storage/byte quota at upload-complete and WebDAV `PUT`, and
set a sane maximum object size.

**Deferred:** tracked as a roadmap item (per-user storage quota), not addressed in this PR.

### 2.7 — LOW — [FIXED] Weak password policy; unverified email updates

`create_user` only rejects empty passwords (no length/complexity floor). `update_me` lets a user
set `email` to any value with no format check, uniqueness check, or verification flow.

Recommendation: add a minimum password length, and validate/verify email changes (at minimum a
format + uniqueness check).

**Fixed (partial):** `create_user` enforces a minimum password length (8) and a syntactic email
check; `update_me` validates the email format. Uniqueness is already enforced by the DB constraint
(`409`). A full email *verification* (confirmation link) flow remains future work.

### 2.8 — INFO — `presign_pictures` returns originals to any token bearer

By design the per-picture token authorizes a presign with no variant restriction, so a leaked
token yields the **original** full-resolution file. This is the intended cross-instance fetch
model; just note that token confidentiality is the entire access control for received pictures.

### 2.9 — INFO — [FIXED] `receive_pictures_announcement` trusts the announced `sender_username`

The `/SharedToMe/<sender>/…` tag is built from the request's `sender_username`, which is not
checked against the `IncomingShare.sender_username` that created the share (only the
*instance* is bound to the JWT). Same-instance only, low impact, but tightening it to the share's
stored sender would remove an avenue for an authenticated peer instance to mislabel the
SharedToMe path.

**Fixed:** `receive_pictures_announcement` now rejects (`401`) when the announced `sender_username`
does not equal the `IncomingShare.sender_username` that created the share.

### 2.10 — INFO — [FIXED] CORS may be wildcard

`build_cors_origin` allows `*` when configured, with `Authorization` permitted and `Any` methods.
Acceptable here because auth is Bearer-in-header (not cookies), so `*` does not leak credentials,
but production should pin `CORS_ORIGINS` to the known frontend origins.

**Fixed:** the backend logs a `WARN` at startup when `CORS_ORIGINS` contains `*`, flagging the
dev-only configuration. (CORS remains permissive when explicitly configured that way.)

---

## 3. Suggested priority

All MEDIUM items and the easy LOW/INFO items were addressed in this PR (§2.1–§2.5, §2.7, §2.9,
§2.10). Remaining future work:

1. **§2.6 — per-user storage quotas** (roadmap): enforce a byte quota at upload-complete and
   WebDAV `PUT`, plus a max object size.
2. **§2.4 follow-up**: optional DNS-rebinding defence (re-check the resolved IP after resolution),
   on top of the `recipient_instance` syntactic validation now in place.
3. **§2.7 follow-up**: a real email-verification (confirmation link) flow.

None of these block correctness of the current authorization model; they further harden
availability and abuse resistance.
