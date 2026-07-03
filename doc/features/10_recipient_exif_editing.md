# Recipient EXIF Editing & Propagation

## 1. Overview & goals

Owners can already edit their pictures' EXIF and have it propagate to recipients
([04_better_exif_support.md](04_better_exif_support.md)). This feature adds the **reverse direction**:

1. **Authorise recipients to edit a shared picture's EXIF** — a per-share permission the owner grants.
2. **Propose-to-owner flow** — an authorised recipient's edit is sent to the owner, who **auto-applies**
   it to the authoritative picture and re-announces, so every recipient converges (the owner is the
   serialization point — last-write-wins, no split brain).
3. **Graceful fallback** — when the share does *not* authorise editing, the recipient keeps the
   **local override** path from [09 §6](09_trash_and_exif_overrides.md): private, DB-only, sticky
   per-field.

This builds directly on the [09 invariant](09_trash_and_exif_overrides.md): a propose-to-owner edit
changes the **owner-authoritative** data (so it propagates to all), whereas a local override stays
private (so it never does). The two are mutually exclusive per field.

**No schema migration of its own** — the permission columns
(`outgoing_shares.allow_exif_edit`, `incoming_shares.allow_exif_edit`) are already in
`001_initial_schema.up.sql` via [09 §4](09_trash_and_exif_overrides.md).

## 2. Decisions (settled)

- **One per-share boolean grant**, `allow_exif_edit`, on the `OutgoingShare`, **propagated** to the
  recipient's `IncomingShare` (display + gate), exactly like `allow_share_back`/`future`. Default
  `FALSE` (no editing).
- **Authorised edits auto-apply at the owner** (no approval queue). Conflicts are resolved by
  last-write-wins because the owner serialises ([09 §2](09_trash_and_exif_overrides.md)). The window
  where two recipients race is the few seconds between an apply and its re-announce — acceptable.
- **Propose ≠ override.** An authorised recipient *proposes*; the owner owns the result. Escalating a
  field to a proposal **clears that field's local override** so the owner's applied value is no longer
  shadowed. A recipient may still choose a private local override even when editing is authorised.
- **The owner re-uses its existing `edit_picture` write-through** ([04 §4](04_better_exif_support.md)):
  DB write + file reconcile + re-announce. A recipient proposal is just another caller of that path.
- **Owned-only fields stay owner-only.** The grant covers EXIF (the [04 §7.3](04_better_exif_support.md)
  editable field set) only; it never permits deletion, tag mutation, or visual edits.

## 3. Permission model

```
OutgoingShare.allow_exif_edit : bool   # owner grants recipients EXIF editing of these pictures
IncomingShare.allow_exif_edit : bool   # propagated copy; drives the recipient UI + server-side gate
```

- Set at share creation and editable later via the share update endpoint; a change re-announces the
  share (same propagation as `name`/`message`/`future` updates) so the recipient's
  `IncomingShare.allow_exif_edit` tracks it.
- Toggling **off** does **not** revert anything already applied (those edits are now the owner's
  authoritative data); it only rejects *future* proposals. Existing recipient **local overrides** are
  untouched (they were never the owner's).

## 4. API

### 4.1 Recipient edit endpoint

`POST /api/authenticated/pictures/{id}/exif` — for a **received** picture. Body carries the
[04 §7.3](04_better_exif_support.md) `set`/`clear` shape plus a mode:

```jsonc
{
  "mode": "propose" | "local",     // default "local"
  "set":   { "gps_lat": 45.92, "gps_lng": 6.87 },
  "empty": ["gps_alt"],            // local mode: claim-as-empty (see §6.3)
  "clear": ["orientation"]
}
```

- `mode: "local"` → write `local_exif_overrides`, recompute the merge, fire the local `metadata`
  event ([09 §6.2](09_trash_and_exif_overrides.md)). Always permitted. Three per-field verbs:
  `set` claims a value, `empty` claims the field as **empty/`null`** (shadows a present owner value
  with emptiness — §6.3), `clear` drops the claim so the owner's value flows through again.
  Overrides are stored as a raw canonical JSON object; a key present with `null` is the empty claim,
  an absent key is un-claimed (`domain::received_exif`).
- In `mode: "propose"` there is no recipient-local `empty`; emptying is proposed to the owner as a
  `clear` (owner-side clear nulls the column — [04 §7.3](04_better_exif_support.md)), so any `empty`
  fields are folded into `clear` before the proposal is sent.
- `mode: "propose"` → **requires** the picture's `IncomingShare.allow_exif_edit = true` (else `403`).
  Send the delta to the owner (§5). On success, **clear those fields from `local_exif_overrides`** so
  the owner's value (arriving via re-announce) is authoritative. Returns `202 Accepted` (the
  authoritative change lands asynchronously via the owner's reconcile + re-announce).

Owned pictures keep using `POST /pictures/{id}/edit` + batch `PATCH /pictures/exif`
([04 §7](04_better_exif_support.md)); this endpoint rejects owned pictures (use the owner path).

### 4.2 Federation verb

`POST /api/federation/pictures/edit_request` (pairwise federation JWT), sent by the recipient's
backend to the owner's:

```jsonc
{
  "picture_id": "<owner's picture id>",
  "requester": "@bob:other.com",
  "set":   { ... },
  "clear": [ ... ]
}
```

Owner-side handler:

1. Resolve the local picture; verify an **active** `OutgoingShare` to `requester` covering it with
   `allow_exif_edit = true` (else `403`). This is the authorisation check — never trust the flag from
   the wire.
2. Validate fields with the [04 §7.3](04_better_exif_support.md) validators (GPS bounds, orientation
   range, set/clear conflict).
3. Apply via the existing `edit_picture` write-through ([04 §4](04_better_exif_support.md)) — DB
   write (bumps `updated_at`, `exif_sync_status = pending`, resets `last_pipeline_run_at`), file
   reconcile job, pipeline wake. The pipeline re-announces the metadata change to **all** recipients
   (incl. the requester) per [04 §10.3](04_better_exif_support.md).

**Same-backend** owner+recipient short-circuits the HTTP call (direct service call), mirroring the
share-announce same-backend path.

## 5. Propagation flow

```
Bob (recipient)                     Alice (owner)                        all recipients
 │  POST /pictures/{id}/exif        │                                    │
 │    mode=propose ───────────────► │                                    │
 │                                  │ verify grant + validate            │
 │                                  │ edit_picture write-through         │
 │                                  │   (DB + file reconcile)            │
 │                                  │ pipeline re-announce ─────────────►│  (incl. Bob)
 │  clear proposed fields from      │                                    │
 │  local_exif_overrides            │                                    │
 ▼  202 Accepted                    ▼                                    ▼  converged
```

- **Convergence / LWW:** because every proposal serialises at the owner, two near-simultaneous
  proposals apply in arrival order; the last committed wins and everyone reconverges on it. No locks.
- **Transitive shares:** a relayer (B in A→B→C) does **not** apply C's proposal — it forwards it
  toward the owner. The picture's `owner` identity is on the row; the proposal is addressed to that
  owner's backend. Relayers are never in the apply path (mirrors the announce/data-fetch rule).

## 6. Edge cases

1. **Share not authorised** (`allow_exif_edit = false`) → `mode: propose` returns `403`; the UI offers
   only local override.
2. **Grant revoked while a proposal is in flight** → the owner's authorisation check (§4.2 step 1)
   rejects it; the recipient keeps whatever local override it had (none, if it cleared on escalate —
   so re-applying as a local override is the recovery).
3. **Field both proposed and locally overridden later** → last action wins on that field: a new local
   override re-shadows; a new proposal re-clears the override. No `Option<Option<T>>` ambiguity
   (reuse [04 §7.3](04_better_exif_support.md) three-state set/clear).
    - **Override-to-empty.** A recipient may shadow a *present* owner value with **emptiness** (e.g.
      strip GPS the owner still carries). Because `local_exif_overrides` is a raw JSON object, this is
      a key present with `null` — distinct from an absent key (un-claimed, owner value flows through).
      The API `empty` list produces the null claim; `clear` removes the key. A sparse `FullExif` cannot
      express this (its `None` is un-claimed), which is why the override store is raw `Value`, not
      `FullExif`. The empty claim is sticky across owner re-announces exactly like a value claim.
    - **One workflow.** Every recipient-override write — the single endpoint, the propose-escalate
      clear, and the feature-14 batch — validates via `domain::validation::validate_exif_edit`, lowers
      its `set`/`empty`/`clear` delta through `received_exif::override_patch`, and applies it with the
      same set-based SQL merge `(local_exif_overrides − clear_keys) ‖ patch` (`patch` carries the
      `null` empty-claims). The stored override additionally drops any set key already equal to the
      owner's value ([09 §6.1](09_trash_and_exif_overrides.md)) — a redundant claim would needlessly
      shadow a future owner edit — without changing the effective columns. The batch endpoint exposes
      `empty` too; for owned pictures and propose-to-owner, `empty` folds into `clear` (nulls the
      column).
4. **Owner offline** → the federation call fails; surface a retryable error; the recipient may fall
   back to a local override meanwhile (and re-propose later).
5. **Picture still in initial extraction** at the owner → reuse [04 §11.2](04_better_exif_support.md)
   `409` ("picture still processing").
6. **Received picture is read-only for non-EXIF ops** — the grant covers EXIF only; deletion stays
   the recipient's local trash ([09 §5.2](09_trash_and_exif_overrides.md)), tags stay local.
7. **Owner's own edit vs a recipient proposal** — identical code path (`edit_picture`); both serialise
   at the owner.

## 7. Documentation updates

- [01_GENERAL_SPECIFICATIONS.md §6](../01_GENERAL_SPECIFICATIONS.md) — add `allow_exif_edit` to the
  share data model and the re-tagging section (recipients may edit EXIF when authorised; propagation
  is owner-driven).
- [03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md) — new `pictures/edit_request` federation
  verb; recipient `POST /pictures/{id}/exif` (`local` vs `propose`); owner re-uses `edit_picture`.
- [06_API_REFERENCE.md](../06_API_REFERENCE.md) — the recipient EXIF endpoint + the federation verb.

## 8. Work breakdown

- [x] Domain/repository: read/write `allow_exif_edit` on `OutgoingShare`/`IncomingShare`; propagate it
  on share create + announce (alongside `name`/`message`/`future`). *(A share-update endpoint does not
  yet exist for the sibling fields either; create + announce propagation is wired now.)*
- [x] Recipient endpoint `POST /pictures/{id}/exif` (`local` → override path; `propose` → gate +
  federation; escalate clears the per-field override). Rejects owned pictures. Supersedes the 09
  `/exif/override` route.
- [x] Federation `pictures/edit_request`: owner-side authorisation (active share + grant), validation,
  `edit_picture` apply, same-backend short-circuit; pairwise-JWT auth + idempotency key field.
- [x] Wire re-announce of the resulting metadata change (provided by
  [04 §10.3](04_better_exif_support.md) — verified it fires for owner-applied recipient proposals via
  `edit_pictures_exif`).
- [x] Frontend: received-picture EXIF editor with "suggest to owner" (propose) vs "just for me"
  (local), reflecting `IncomingShare.allow_exif_edit`; three per-field verbs `set`/`empty`/`clear`
  (`empty` claims a field as empty/`null` — `useExifDraft.buildPayload`). *(Per-share "allow
  recipients to edit EXIF" toggle in the share dialog: still pending.)*
- [x] Tests (`tests/recipient_exif_editing.rs`): grant gates propose (403 when off); proposal applies
  at owner + re-announces to the requester; escalate clears override; grant-revoked-in-flight
  rejected; uncovered requester rejected; owned-picture proposal rejected; same-backend short-circuit.
- [x] Docs (§7).
