# Picture Creator

## 1. Overview & goals

Today a picture's only human identity is its **owner** — the user whose library holds it. But
ownership and authorship are different: you routinely own pictures you did not create — a friend's
camera roll you extracted yourself, a shared family camera, a photo someone handed you, or (feature

27) an anonymous contribution to a public album. This feature adds an editable, propagated
    **creator** attribution field, distinct from the owner.

**Goals:**

- A single `creator` credit on every picture, defaulting to the owner.
- Carried across share announcements **and re-announcements**, so recipients see the real author.
- **Locally overrideable** by a recipient (their own view only), never propagated.
- A lightweight **format convention** distinguishing Archypix identities, anonymous public
  uploaders, and free-text credits.
- Consumed by feature 27: anonymous uploads stamp the uploader's entered name.

**Explicitly out of scope — and deliberately *not* generalized.** Creator looks superficially like
other "metadata" fields, but each has a genuinely different policy, so there is **no shared
override framework** (attempting one costs more than three purpose-built fields):

| Field                    | Policy                                                                                                                                   | Home                               |
|:-------------------------|:-----------------------------------------------------------------------------------------------------------------------------------------|:-----------------------------------|
| **caption**              | every user's caption is visible (per-user, multi-valued) — a *ratings-like* annotation                                                   | future per-user-annotation feature |
| **filename**             | local; fixed at first announcement, **never** refreshed by later re-announcements; plain editable column (+ future auto-rename services) | no override layer                  |
| **ratings / face-names** | per-user annotations with their own cross-instance visibility story                                                                      | future feature                     |
| **ML vectors**           | computed, per-producer + model-version, propagate-to-cache (don't recompute)                                                             | own JSONB, wired when ML lands     |

Only **creator** has the "owner-authoritative + recipient local override + propagate" shape, and it
is the only field this feature builds.

---

## 2. Decisions

- **Independent of EXIF.** `Exif.Image.Artist` / IPTC By-line is empty on the phone-photo majority
  (iOS and stock Android never set it) and only opt-in on higher-end cameras, so it is not a reliable
  creator source. Creator is an app-level field — not derived from, nor written back to, file EXIF.
  (A best-effort "seed the default from `Artist` when the file happens to carry one" is a possible
  later nicety, off by default.)
- **Single free-text credit, not a structured identity.** Creator is pure attribution — it is
  **never an authorization principal** (it grants no access), so it needs no resolvable identity and
  may hold arbitrary strings. A format convention (§3) gives it structure where it matters.
- **`NULL` means "the owner".** An unset creator resolves to the owner's `@username:domain` on read.
  No backfill, no global-domain-in-migration problem, and the feature-27 contribution query
  (`creator LIKE '#%'`) naturally excludes owner-default rows.
- **Sigils are system-owned.** A manual edit may not begin with `@` or `#` (rejected), so a user
  cannot forge a fake identity or a fake anonymous credit.
- **Local override now; propose-to-owner deferred (phase 2).** A recipient may relabel their own view
  (DB-only, never propagates, not even transitively). Propagating a recipient's correction back to the
  owner — mirroring the feature-10 EXIF propose flow — is designed here (§7) but built as a follow-up.

---

## 3. Format convention

The `creator` string is interpreted by its leading sigil:

| Form           | Meaning                                         | Set by                                   | UI                          |
|:---------------|:------------------------------------------------|:-----------------------------------------|:----------------------------|
| `@user:domain` | a verified Archypix identity                    | default (= owner); propagated downstream | link to the profile         |
| `#name`        | an unauthenticated public uploader's typed name | feature-27 anonymous upload              | plain, "contributed by"     |
| plain text     | an arbitrary manual credit (`Grandpa's camera`) | owner editing                            | plain                       |
| `NULL`         | the owner (unset)                               | ingest default                           | resolved to `@owner:domain` |

**Parsing** is trivial and total: starts with `@` and contains `:` → identity (linkify); starts with
`#` → anonymous name; otherwise → plain credit.

**Sigil guard.** When a user manually edits creator (owned value or recipient override), the input is
rejected if it begins with `@` or `#`. The system owns those sigils — only the ingest default emits
`@…:…` and only feature-27 uploads emit `#…`.

---

## 4. Schema changes

```sql
ALTER TABLE pictures ADD COLUMN creator          TEXT;  -- NULL ⇒ owner default
ALTER TABLE pictures ADD COLUMN creator_override  TEXT;  -- recipient-local (received pictures only)
```

- **`creator`** — for an owned picture, the owner's value (`NULL` ⇒ derived owner identity); for a
  received picture, the origin's propagated, already-resolved value.
- **`creator_override`** — the recipient's local relabel; only meaningful on received pictures.
- **Displayed creator** = `coalesce(creator_override, creator, owner_identity())`.

No backfill is required (`NULL` already means the owner). This mirrors the feature-09 EXIF split
(`remote_exif_data` origin + `local_exif_overrides` recipient) but for a single scalar, so no
JSONB and no per-field merge — just the coalesce.

---

## 5. Resolution & the owner default

The owner identity is derived on read from the local user's `username` + the instance
`global_domain` (config), rendered `@username:global_domain`. Two consequences:

- A `NULL` creator never leaves the backend as `NULL`: announcements (§6) and API reads resolve it to
  the concrete owner identity first.
- If the owner's user row is gone (deleted account), the raw stored `owner_username`/domain (received
  pictures) or a neutral placeholder is shown rather than erroring.

---

## 6. Propagation

- **`AnnouncedPicture`** (`clients/federation/models.rs`) gains `creator: String`. The sender resolves
  `NULL → @owner:domain` before announcing, so the wire value is always concrete.
- **Recipient side** (`create_received`): store the announced value into `creator`; **preserve any
  `creator_override`** across re-announcements — exactly as `create_received` preserves
  `local_exif_overrides` while refreshing `remote_exif_data`.
- **The local override never leaves the recipient.** A transitive re-share carries the *origin's*
  creator (the `creator` column, not the relayer's `creator_override`).
- **Physical copy (feature 11).** A new owned copy carries the **source picture's** creator —
  attribution travels with the content, it is not reset to the copier. (A copy of a `#name`
  contribution keeps `#name`; a copy of Alice's `@alice:…` photo keeps `@alice:…`.)

---

## 7. API

**`POST /api/authenticated/pictures/{id}/creator`** — body `{ "value": string | null, "mode"?: "local" | "propose" }`.

| Picture  | `mode`            | Effect                                                                                           |
|:---------|:------------------|:-------------------------------------------------------------------------------------------------|
| Owned    | (ignored)         | Set `creator`. `value` null/empty ⇒ reset to owner default (`creator = NULL`).                   |
| Received | `local` (default) | Set `creator_override`. `value` null ⇒ clear the override (reset to origin).                     |
| Received | `propose`         | **Phase 2** — propose to the owner (grant-gated, mirrors feature 10). Returns `403` until built. |

- **Validation:** reject a manual `value` that begins with `@` or `#` (§3 sigil guard).
- **Owned edit** bumps `updated_at` and wakes the pipeline, so the change re-announces to recipients
  through the normal announcement-delta path (the announcement backstop already re-dirties tracking
  rows whose `announced_updated_at` trails `updated_at`).
- **Reads:** the resolved/materialized creator is added to the picture-detail and list projections.
- **Batch:** creator fits the feature-14 batch-edit surface (future); the single-picture edit is the
  core delivered here.

---

## 8. Frontend

- **Info panel:** a "Created by {creator}" field. `@user:domain` renders as a profile link; `#name`
  renders as "contributed by {name}"; plain text renders verbatim.
- **Inline edit:** owned pictures edit the authoritative `creator` (with a "reset to owner" affordance
  when set); received pictures edit the local `creator_override` (with "reset to original" when
  overridden). The sigil guard is enforced client-side too, with the server as the authority.
- Batch-editable later via the feature-14 panel.

---

## 9. Edge cases

- **Owner default resolution:** `NULL` creator ⇒ `@owner:domain`; the contribution query
  (`creator LIKE '#%'`) correctly skips owner-default rows.
- **Sigil forgery:** manual `@…`/`#…` input rejected; only system paths emit sigils.
- **Override never propagates**, including transitively — downstream recipients always see the origin's
  creator.
- **Copy carries the source creator**, not the copier (attribution travels).
- **Relayed contribution:** a received picture whose origin creator is itself `#name` (a share or copy
  of a public contribution) propagates that `#name` verbatim.
- **Deleted/unresolvable owner:** fall back to the stored raw username or a neutral placeholder rather
  than failing the read.

---

## 10. Doc updates

- `01_GENERAL_SPECIFICATIONS.md` §1/§6 — note the owner-vs-creator distinction and creator propagation.
- `03_BACKEND_ARCHITECTURE.md` — `pictures.creator`/`creator_override`, `AnnouncedPicture.creator`,
  the creator edit endpoint, and its re-announce-on-edit behaviour.
- `06_API_REFERENCE.md` — the `POST /pictures/{id}/creator` endpoint + the `creator` response field.
- `05_FRONTEND_ARCHITECTURE.md` — the info-panel creator field.

---

## 11. Work breakdown

**Phase 1 (this feature):**

- [ ] Migration: `pictures.creator`, `pictures.creator_override`; regenerate `schema.sql` + `sqlx prepare`.
- [ ] Domain: creator parse/format helpers + sigil-guard validation in `domain::picture`/`domain::tag`.
- [ ] Owner-identity resolution helper (`@username:global_domain`), used on read + before announce.
- [ ] `AnnouncedPicture.creator`; sender resolves `NULL`→owner; `create_received` stores it and
  preserves `creator_override`.
- [ ] `copy_picture` carries the source creator.
- [ ] `POST /pictures/{id}/creator` (owned set / received local override); re-announce on owned edit.
- [ ] Add resolved creator to detail + list projections.
- [ ] Frontend info-panel field (linkify / edit / reset).
- [ ] Tests: default resolution, sigil guard, propagation + override preservation, transitive
  no-leak, copy-carries-creator.

**Phase 2 (soon):** propose-to-owner — a grant (reuse `allow_exif_edit` as a metadata-edit grant, or a
dedicated `allow_creator_edit`) + a federation verb mirroring feature 10's `pictures/edit_request`;
escalating a field to a proposal clears its local override.
