# Public Shares

## 1. Overview & goals

A **public share** is a capability link that grants unauthenticated access to the pictures under a
tag, served entirely by the owner's backend. It is the first *pull* share in the system: every share
today is **push** (the owner announces per-picture tokens to a *named recipient backend*, which stores
an `IncomingShare` and drives the pipeline). A public share has **no recipient backend and no
`IncomingShare`** — coverage is computed **live at request time**, like the hierarchy resolver, not
announced. That single decision keeps public *viewing* out of the pipeline / `share_announcements` /
federation entirely.

The roadmap line is three sub-features with three mechanics:

| Sub-feature                       | Mechanic                                                                                                      | Reuses                                                                                   |
|:----------------------------------|:--------------------------------------------------------------------------------------------------------------|:-----------------------------------------------------------------------------------------|
| **View**                          | link token → live tag-coverage query → presign, all on the owner backend                                      | `presign_picture_variant` (owned + received-proxy), `push_filters`                       |
| **Contribute** (anonymous upload) | anon upload → picture *owned by the creator*, tagged into the share, `creator = #name`                        | `begin_upload_batch` (dedup-reject), `services::storage`, `infra::ratelimit`, feature 26 |
| **Convert** (authed visitor)      | recipient-*initiated* subscribe → a **derived** `OutgoingShare` → existing push machinery; **or** save copies | the whole share pipeline, feature 11                                                     |

Only **Convert** re-enters the pipeline. That is the clean seam.

**Depends on feature 26** (picture creator): anonymous uploads set `creator = "#" + entered_name`, and
that plus tag membership *replaces* any contributions table.

---

## 2. Decisions

- **Live coverage, not announcements.** Public viewing resolves coverage on each request
  (`picture tag <@ share.tag_path`, `local_user_id = owner`, not deleted, not hidden-dedupe). No
  per-picture tokens, no tracking table, no pipeline involvement. Revocation is therefore instant.
- **Separate `public_shares` table**, not an overloaded `outgoing_shares`. The latter's
  `recipient_*`, `shareback_of`, retry/backoff, and announce state machine are all irrelevant to a
  link.
- **One `allow_originals` permission tier.** Download, "save a copy", and "convert to a share" are all
  *original-extraction* vectors, so they ride a single toggle. `OFF` ⇒ a **view-only gallery**:
  thumbnails only (which carry no EXIF), with EXIF/GPS also stripped from the JSON payload.
- **Anonymous contributions are rejected on dedup, never hidden.** A contributed picture is real bytes
  with weight on the creator's storage, unlike a *received* picture that lives on the sender's side.
  So uploads run through the existing upload-time dedup (`begin_upload_batch`) and a hash hit against
  the creator's live **or trashed** pictures is **rejected before storage** — the content-dedup
  reconciler / boomerang guard (feature 11) stays a received-picture mechanism only.
- **Authed contribution is ShareBack, not upload.** An anonymous visitor uploads bytes; an
  authenticated visitor "uploads" by **sharing a tag back** — their pictures stay on *their* storage
  and arrive as *received* pictures (unbilled), attributable to a real account, revocable by them.
  Pure reuse of the ShareBack machinery (feature 01 §7.3).
- **Convert is recipient-initiated.** A new federation verb (`shares/public/claim`) lets a visitor's
  backend pull a public share into a real derived share — the reverse of today's sender-initiated
  shares.
- **No contributions table.** Feature-26 `creator` (`#name`) + tag membership fully identify
  contributions; the source share is the tag, not a stored id. Abuse tracing is by creator-name or by
  tag (no IP retention) — accepted trade-off.

---

## 3. Schema changes

**New table `public_shares`** (owner's backend only):

```sql
CREATE TABLE public_shares
(
    id                   uuid PRIMARY KEY             DEFAULT uuid_generate_v4(),
    owner_id             uuid                NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    tag_path             ltree               NOT NULL,                  -- what's covered
    name                 varchar(64)         NOT NULL,
    message              text,
    token                text                NOT NULL UNIQUE,           -- 256-bit base64url secret in the URL
    password_hash        text,                                          -- optional access gate
    expires_at           timestamptz,                                   -- optional
    allow_originals      boolean             NOT NULL DEFAULT false,    -- download + copy + convert-to-share
    allow_upload         boolean             NOT NULL DEFAULT false,    -- anonymous contribution
    allow_share_back     boolean             NOT NULL DEFAULT false,    -- authed ShareBack (forced on if allow_upload)
    conv_allow_exif_edit boolean             NOT NULL DEFAULT false,    -- inherited by the derived share on Subscribe
    conv_future          boolean             NOT NULL DEFAULT true,     -- inherited by the derived share on Subscribe
    status               public_share_status NOT NULL DEFAULT 'active', -- active | revoked
    created_at           timestamptz         NOT NULL DEFAULT now(),
    revoked_at           timestamptz
);
CREATE INDEX idx_public_shares_owner ON public_shares (owner_id);
CREATE INDEX idx_public_shares_tag ON public_shares USING gist (tag_path);
-- token lookup is by the UNIQUE constraint.
```

**Modified `outgoing_shares`**: `+ derived_from_public_share_id uuid REFERENCES public_shares(id) ON DELETE SET NULL`
— provenance for a **derived share** (minted on Subscribe) and the key for the revoke-time cascade prompt.

*(Expiry is checked live on every request; `status` need not be swept to a terminal value.)*

---

## 4. Permission model

Two directions, gated independently:

|                                        | Action                                                       | Gate                                                 |
|:---------------------------------------|:-------------------------------------------------------------|:-----------------------------------------------------|
| **OUT** (take pictures from the album) | download original · save a copy · convert to a derived share | **`allow_originals`**                                |
| **IN** (contribute to the album)       | anonymous upload (bytes → creator's library)                 | `allow_upload`                                       |
|                                        | authed ShareBack (a tag stays on the contributor's side)     | `allow_share_back` (**forced on if `allow_upload`**) |

`allow_originals = false` ⇒ **view-only gallery**: thumbnails only, EXIF/GPS omitted from the JSON;
none of download / copy / convert are offered (convert and view-only are mutually exclusive). The
create dialog warns: *"Viewers won't be able to see or download original files — only the thumbnails
shown, with EXIF/GPS removed."*

**Convert sub-options** (apply to the *derived share* Subscribe mints; irrelevant to one-shot
download/copy):

- `conv_allow_exif_edit` → derived share's `allow_exif_edit` (feature 10).
- `conv_future` → derived share's `future`.
- `allow_share_back` → the convert menu offers "convert **+** share back a tag".

**Coherent corner:** `allow_upload` on with `allow_originals` off is a valid *drop-only* gallery
(contribute in, nothing out); ShareBack (an inbound action) is still offered there.

---

## 5. URL & discovery

```
https://<frontend>/s/<global_domain>/<username>/<token>
```

The URL carries everything a resolver pass needs, so **any** correctly-CORS'd frontend can open it: it
resolves `(username, global_domain)` → backend via the resolver, then calls the backend's public
endpoints with `token`. The share is tag-anchored (the creator is shown as a display name on the page,
not required for access). The `token` is a 256-bit unguessable secret — a public share is **link +
token (+ optional password) gated**, not world-visible.

---

## 6. Flow — view (unauthenticated)

All on the **owner's** backend, under `/api/public/*` (no user JWT):

- `GET /api/public/shares/{token}` → `{ name, message, owner_display, permissions, picture_count, requires_password, expires_at }`.
- `POST /api/public/shares/{token}/unlock { password }` → a **`public_share` JWT** (new
  `common::auth` token type, claim `{ share_id }`, short TTL) sent on subsequent calls. Only for
  password-gated shares. (A JWT beats re-running argon2 on every thumbnail fetch.)
- `GET /api/public/shares/{token}/pictures?cursor=` → paginated picture metadata (+ presigned
  thumbnails). **View-only** shares omit `captured_at` / `gps_*` / `exif_data` from the payload.
- `GET /api/public/shares/{token}/pictures/{pid}/url?variant=` → **coverage-checked** presign:
  the picture's tag `<@ share.tag_path`, `local_user_id = owner`, not deleted, not hidden-dedupe; a
  view-only share only presigns thumbnail variants. **Received** pictures in coverage proxy to the real
  owner via `presign_remote_pictures` + the stored `tags.picture_token` — the same owned/received
  branching `presign_picture_variant` already implements, with the ownership check swapped for the
  coverage check.

Authorization = the token (+ optional password JWT + optional `expires_at`), re-validated per request.

---

## 7. Flow — anonymous contribution (`allow_upload`)

- `POST /api/public/shares/{token}/uploads { contributor_name, files: [...] }` → presign staging.
  Enforces the creator's **storage quota** (`services::storage`, charged to the owner), size/count/MIME
  caps, and per-IP + per-share **rate limits** (`infra::ratelimit`). Runs the existing **upload-time
  dedup** (`begin_upload_batch`): a hash match against the owner's live **or trashed** pictures is
  **rejected** (not stored, not hidden — the bytes are weighty).
- `POST /api/public/shares/{token}/uploads/{pid}/complete` → server-side copy, assign the share's tag
  (`source = manual`), set **`creator = "#" + contributor_name`** (feature 26), trigger the pipeline
  (`ingest`) → thumbnails run under the owner's ownership.

**Contributions are fully derived:** owned pictures (`remote_picture_id IS NULL`) under the share's
tag with `creator LIKE '#%'`. "Bulk-remove a contributor" = the same, filtered by the `#name`. No
table, no stored source share, no IP.

---

## 8. Flow — convert (authenticated visitor, requires `allow_originals`)

A menu of up to three actions:

| Action                         | Extra gate         | Effect                                                                                                                                                   |
|:-------------------------------|:-------------------|:---------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Save a copy**                | —                  | feature-11 physical copies into the visitor's library (cross-instance byte copy, `copy_source_*` root-resolved, feature-26 creator carried from source). |
| **Convert → derived share**    | —                  | a real `OutgoingShare` (owner→visitor), optional local-tag mapping.                                                                                      |
| **Convert + share back a tag** | `allow_share_back` | the above **plus** a reciprocal share of a chosen tag back to the owner, auto-accepted.                                                                  |

**Subscribe (recipient-initiated).** The visitor's backend calls a new verb on the owner's backend:
`POST /api/federation/shares/public/claim { token, requester_username, requester_instance }` (pairwise
federation JWT, visitor→owner). The owner validates the token is `active` + `allow_originals`, mints an
`OutgoingShare` (`recipient = visitor`, `tag = share.tag_path`, `derived_from_public_share_id`,
`allow_share_back = allow_share_back`, `allow_exif_edit = conv_allow_exif_edit`, `future = conv_future`,
status `pending_first_announcement`), and **returns the metadata** rather than calling back (federation
rule 2 — no callback into the visitor's uncommitted state). The visitor's backend creates the
`IncomingShare` (`active`); the owner's pipeline announces coverage from there. **Same-backend** visitor
short-circuits federation.

**Convert + share back.** Because the derived share carries `allow_share_back` from the public share,
the visitor's reciprocal `OutgoingShare` (`shareback_of` = derived share) auto-accepts on the owner
(feature 01 §7.3, §6.5), and its mapping lands the contributor's pictures under the **public share's
tag** → they appear in the gallery (as *received* pictures, presign-proxied), at zero storage cost to
the owner. Soft consent note in the UI: *"Pictures you share back will be visible in this album to
everyone with the link."*

---

## 9. Revocation

`POST /api/authenticated/shares/public/{id}/revoke` → `status = revoked` → the live coverage check
fails **instantly**. (Already-issued presigned S3 URLs live until their short TTL — inherent; there
are no per-picture tokens to purge.)

- **Cascade prompt:** the creator is asked *"also revoke the N derived shares created from this link?"*
  (`derived_from_public_share_id` makes it one query); if yes, each derived `OutgoingShare` goes through
  the normal revoke path.
- **Revoke ≠ delete contributions** — contributed pictures are the creator's own now. A separate
  "revoke + trash contributions" action trashes owned pictures under the tag with `creator LIKE '#%'`.
- **Paired ShareBacks are out of scope here** — a derived share's reciprocal ShareBack is the *visitor's*
  own `OutgoingShare`; tearing it down on revocation is a ShareBack-feature concern (a possible future
  "audit + auto-revoke sharebacks"), not this feature.

---

## 10. Received pictures in a public share (allowed, with warning)

A public share may cover *received* pictures (content the creator got via a private share). Presign
proxies to the real owner as in §6. The create dialog **warns** that this re-exposes another user's
content. Two documented residuals: the upstream owner has no visibility or veto (the warning is the
only control), and "save a copy" of received content is the deepest escalation (an independent owned
copy of privately-shared content). If the upstream owner later revokes, the proxy token dies and the
picture drops from the gallery; copies already taken are independent.

---

## 11. Federation protocol

**New verb `POST /api/federation/shares/public/claim`** (visitor's backend → owner's backend, pairwise
JWT): body `{ token, requester_username, requester_instance }`. The owner validates `active` +
`allow_originals`, mints the derived `OutgoingShare` (§8), and returns `{ name, message, tag_path,
allow_share_back, allow_exif_edit, future }`. Returns `404` for an unknown/revoked/expired token or when
`allow_originals` is false.

All other convert machinery reuses existing verbs: `pictures/announce` (derived-share announcement),
`shares/announce` + ShareBack (the reciprocal share), `pictures/presign` (blob fetch).

---

## 12. Services / repositories / API surface

- **`domain::public_share`** — `PublicShare`, `PublicShareStatus`, token generation, permission
  helpers, coverage predicate builder (reuses `push_filters` with owner scope, no user context).
- **`repository::public_share`** — CRUD, `find_by_token`, `list_by_owner`, coverage/list queries,
  `find_derived_shares(public_share_id)`.
- **`services::shares::public`** — create/update/revoke; the unauth view/list/presign path; the
  contribution upload path (wrapping `begin_upload_batch` + `complete_upload` with public-share auth +
  creator stamping); the convert paths (copy / claim / claim+shareback).
- **`services::federation`** — `receive_public_claim` (mint derived share, return metadata).
- **API:** authenticated `/api/authenticated/shares/public` (create/list/update/revoke); public
  `/api/public/shares/{token}[/unlock|/pictures|/pictures/{id}/url|/uploads|/uploads/{id}/complete]`;
  federation `/api/federation/shares/public/claim`. The public routes mount in `user::public_routes`.
- **`common::auth`** — new `TokenType::PublicShare` (claim `{ share_id }`, short TTL).

---

## 13. Abuse controls & config

Anonymous upload is the exposed surface. All values are runtime settings (feature-23 `common::settings`
engine) with sane defaults, quota math via `services::storage` charged to the creator:

- per-IP + per-share upload **rate limits** (`infra::ratelimit`).
- `public_upload_max_file_bytes` (≈100 MB), `public_upload_max_files_per_request` (≈50).
- MIME allowlist = the existing ingestable image/video set.
- optional per-share hard byte cap (default none — the creator's storage quota is the real ceiling).
- `public_share_session_ttl_secs` for the password JWT.

Public pages are `noindex`; token lookup is constant-time-ish via the unique index.

---

## 14. Edge cases

- **Tag rename cascade** must also rewrite `public_shares.tag_path` (like `outgoing_shares`).
- **Contributions surface in the creator's own views** — hierarchies, WebDAV, and any `future = true`
  `OutgoingShare` on the same tag (the tag is a normal tag). Documented, not prevented.
- **Upload dedup-reject** (not hide): a byte-dupe of the creator's existing *or trashed* picture is
  rejected at upload — including content the creator previously trashed (a boomerang-style protection,
  enforced by rejection because the bytes are weighty).
- **Revocation is instant** for coverage; issued presigned URLs expire on their own TTL.
- **View-only + upload** = drop-only gallery (coherent); **convert requires `allow_originals`** so a
  view-only link cannot be converted.
- **Multiple public shares per tag** are allowed and independent.
- **Password change** invalidates old sessions only best-effort (short JWT TTL); no denylist.
- **Offline owner** ⇒ received pictures in the share aren't fetchable (inherent decentralised limit).

---

## 15. Frontend

- **Creator's management UI** (shares area): a "Public links" section — create (tag picker + the §4
  toggles + password/expiry), list, copy link, revoke (with the cascade prompt), derived-share count,
  and a **contributions view** (owned pictures with `#` creator under the tag) with bulk-delete by
  contributor name.
- **Public page** — route `/s/:global_domain/:username/:token`: resolves the backend, renders the
  gallery; password gate when required; download/copy/convert controls per permissions; an upload
  widget (contributor name + files) when `allow_upload`.
- **Authed visitor** — the convert menu (save a copy / subscribe / subscribe + share-back with a tag
  picker and the consent warning).

---

## 16. Testing

- **View:** token resolves; coverage-checked presign for owned and received (proxied) pictures;
  view-only strips originals + EXIF/GPS from bytes and JSON; revoked/expired/wrong-password rejected.
- **Contribution:** anonymous upload lands owned + tagged + `creator = #name`; quota + caps + rate
  limits enforced; **dedup-reject** against live and trashed; contributions query by tag + `#`.
- **Convert:** same-backend and cross-instance subscribe (recipient-initiated `claim`); derived share
  inherits `conv_*`; save-a-copy carries creator + `copy_source_*`; subscribe + share-back auto-accepts
  and lands under the public tag.
- **Revocation:** instant coverage cut; cascade prompt revokes derived shares; contributions survive
  unless "trash contributions" chosen.
- **Security:** unguessable token; no cross-share leakage; received-picture proxy honours upstream
  revocation.

---

## 17. Doc updates

- `01_GENERAL_SPECIFICATIONS.md` §6 — add public shares (link-gated pull share; view/contribute/convert).
- `02_INFRASTRUCTURE_DESIGN.md` — the public link URL shape + resolver pass.
- `03_BACKEND_ARCHITECTURE.md` — `public_shares` table, the `/api/public/shares/*` surface, the
  `shares/public/claim` verb, `TokenType::PublicShare`, the recipient-initiated convert seam.
- `06_API_REFERENCE.md` — all new endpoints.
- `05_FRONTEND_ARCHITECTURE.md` — the public page route + the "Public links" management surface.

---

## 18. Work breakdown

- [ ] Migration: `public_shares` (+ `public_share_status` enum), `outgoing_shares.derived_from_public_share_id`; regen `schema.sql` + `sqlx prepare`.
- [ ] `domain::public_share` + `repository::public_share` (CRUD, `find_by_token`, coverage/list, derived-share lookup).
- [ ] `common::auth` `TokenType::PublicShare`.
- [ ] `services::shares::public`: create/list/update/revoke (+ cascade prompt).
- [ ] Public view surface: `GET shares/{token}`, `unlock`, `pictures`, `pictures/{id}/url` (coverage-checked presign reusing owned/received branching;
  view-only stripping).
- [ ] Contribution: `uploads` + `complete` wrapping `begin_upload_batch`/`complete_upload` with public auth, quota/caps/rate-limit, `creator = #name`.
- [ ] Convert: save-a-copy (feature 11 from a public link); `shares/public/claim` verb + `receive_public_claim` (derived share); subscribe +
  share-back wiring.
- [ ] Abuse-control settings (feature-23 keys) + defaults.
- [ ] Frontend: public page (gallery / password / download / copy / convert / upload) + creator's "Public links" management + contributions
  moderation.
- [ ] Tag-rename cascade includes `public_shares.tag_path`.
- [ ] Tests (§16).
