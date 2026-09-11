# Better Sharing Support

## 1. Overview

This feature completes the sharing system by activating the machinery behind already-present but dormant fields (`future`, `allow_share_back`,
`shareback_of`) and filling in the gaps identified by a codebase audit. It touches the pipeline, the federation protocol, the presign security model,
and several service flows.

**Goals:**

- Automatically announce pictures to share recipients when a picture enters a shared tag (the `future = true` path), and unannounce when it leaves.
- Implement ShareBack: Bob can share back to Alice, who auto-accepts and gets an automatic `SharedTagMappingService` rule pointing to her original
  tag.
- Enforce loop prevention on both sender and recipient sides.
- Make transitive sharing and presigning work end-to-end, with token refresh when a picture's upstream share is partially revoked.
- Auto-revoke Bob's downstream share when Alice's upstream revocation removes all pictures from a directly re-shared `SharedToMe` tag.
- Protect the `SharedToMe` tag prefix from manual use.
- Replace the coarse per-share presign token with a per-picture token model for correct revocation semantics.

---

## 2. Key Design Decisions

**Per-picture presign tokens (replaces `OutgoingShare.share_token`).**
The old model authorised presigning any of Alice's pictures if *any* active `OutgoingShare` held the presented token. This allowed a still-active
share B token to presign pictures that were only accessible via a now-revoked share A. The new model stores one unique token per
`(outgoing_share, picture)` row in a `share_announcements` tracking table. Revoking a share deletes its tokens immediately. The presign endpoint
resolves the token directly to a picture — no share ID or picture ID needed in the request.

**Tracking table as the announce/unannounce source of truth.**
`share_announcements(outgoing_share_id, picture_id, picture_token)` on the sender's side records what has been announced. The pipeline diffs current
coverage against this table. No separate recipient-side tracking table is needed — the recipient's `pictures` + `tags` tables already provide
equivalent information.

**Tokens stored in the `tags` table for received pictures.**
The recipient stores the `picture_token` on the `incoming_share` source tag row (not on the `pictures` row), because a picture can be received via two
concurrent `IncomingShare`s (deduplication), each with its own token. Storing per-tag allows correct token selection (pick any active one,
`ORDER BY source_id` for determinism) and correct behaviour when one share is partially revoked while another remains.

**Pipeline as the announce/unannounce driver (not API hooks).**
All tag changes (manual, service config, inbound share registration) already wake the pipeline. The announcement step runs after tag reconciliation
for each batch of dirty pictures. This single convergence point handles all paths without duplicating logic.

**`cleanup_incoming_share` handles transitive unannounce for deleted pictures.**
When a received picture is deleted (no remaining `incoming_share` tags), the pipeline will never process it again. `cleanup_incoming_share` directly
queries `share_announcements` for downstream recipients of the deleted picture and enqueues `UnannounceSharedPictures` tasks before deleting the
picture rows.

**One `SharedTagMappingService` with `n` mappings per user.**
Auto-created ShareBack mappings are added to the user's existing `SharedTagMappingService` (or a new one is created if none exists). This matches the
spec design and avoids a growing list of single-mapping services.

**Transitive revocation is tag-prefix scoped.**
When Bob receives a revocation from Alice for `IncomingShare is-001` (tag `SharedToMe.alice_AT_...Travel.France`), Bob finds all his `OutgoingShare`s
whose `tag_path` is exactly or prefixed by `SharedToMe.alice_AT_...Travel.France` and auto-revokes them. Mixed-tag shares (e.g. Bob sharing
`/Photos/Holidays/2024` that includes Alice's pictures via `SharedTagMappingService`) are not affected — the tracking table handles picture-level
cleanup for those.

---

## 3. Database Schema Changes

All changes go into `back/migrations/001_initial_schema.up.sql` (the single migration file). Reset with
`cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare`.

### 3.1 New table: `share_announcements`

```sql
CREATE TABLE share_announcements (
    outgoing_share_id UUID    NOT NULL REFERENCES outgoing_shares(id) ON DELETE CASCADE,
    picture_id        UUID    NOT NULL,  -- sender's local picture.id
    picture_token     UUID    NOT NULL DEFAULT gen_random_uuid(),
    PRIMARY KEY (outgoing_share_id, picture_id),
    UNIQUE (picture_token)
);
CREATE INDEX idx_share_announcements_picture ON share_announcements(picture_id);
```

The `ON DELETE CASCADE` ensures tracking rows are cleaned up automatically if an `OutgoingShare` is hard-deleted (not currently done, but defensive).

The `UNIQUE (picture_token)` allows the presign endpoint to resolve a token to a picture in O(1) without knowing the `outgoing_share_id` or
`picture_id`.

### 3.2 Modified table: `outgoing_shares`

**Remove column:** `share_token UUID NOT NULL DEFAULT gen_random_uuid()`.

This per-share bearer token is replaced entirely by per-picture tokens in `share_announcements`. Also remove the associated Redis presign-token cache
logic referencing it.

### 3.3 Modified table: `incoming_shares`

**Remove column:** `origin_share_token UUID` — replaced by per-picture tokens in the `tags` table.

**Add column:** `allow_share_back BOOLEAN NOT NULL DEFAULT FALSE` — propagated from the sender's `ShareAnnouncement` at creation time; lets the
frontend show or hide the "Share back" button.

### 3.4 Modified table: `tags`

**Add column:** `picture_token UUID` (nullable) — only populated for `source = 'incoming_share'` rows. Stores the token Alice generated in
`share_announcements` and sent in `AnnouncedPicture`. Used by Bob's backend to authorise presign calls to Alice on Bob's clients' behalf, and
forwarded to Carol in transitive announcements.

```sql
ALTER TABLE tags ADD COLUMN picture_token UUID;
CREATE UNIQUE INDEX idx_tags_picture_token ON tags(picture_token) WHERE picture_token IS NOT NULL;
```

The partial unique index enforces global token uniqueness and makes the lookup fast.

**Modify `assign_incoming_share_tag` conflict behaviour** from `ON CONFLICT DO NOTHING` to
`ON CONFLICT DO UPDATE SET picture_token = EXCLUDED.picture_token`. This allows re-announcements (token refresh) to update the stored token without
needing a separate update query.

---

## 4. Domain Type Changes

**`OutgoingShare`** (`domain/share.rs`): remove `share_token: Uuid`.

**`IncomingShare`** (`domain/share.rs`): remove `origin_share_token: Option<Uuid>`, add `allow_share_back: bool`.

**`ShareAnnouncement`** (`clients/federation/mod.rs`): remove `share_token: Uuid` (tokens are now per-picture in `AnnouncedPicture`). Fields
`allow_share_back: bool` and `shareback_of: Option<Uuid>` were already present.

**`AnnouncedPicture`** (`clients/federation/mod.rs`): add `picture_token: Uuid`. For pictures owned by the sender, the sender generates this from
`share_announcements`. For pictures the sender received (transitive sharing), the sender copies the `picture_token` from the relevant `incoming_share`
tag row.

**New: `PicturesUnannouncement`** (`clients/federation/mod.rs`):

```rust
pub struct PicturesUnannouncement {
    pub outgoing_share_id: Uuid,
    pub sender_username:   String,
    pub sender_instance:   String,
    pub picture_ids:       Vec<String>,  // sender's local picture IDs as strings
}
```

---

## 5. Per-Picture Token Model (Presign Rework)

### 5.1 Token lifecycle

| Event                                                    | Action                                                                                       |
|----------------------------------------------------------|----------------------------------------------------------------------------------------------|
| Picture enters an OutgoingShare (pipeline announce step) | `INSERT INTO share_announcements(outgoing_share_id, picture_id)` → `picture_token` generated |
| Picture is announced to recipient                        | `picture_token` included in `AnnouncedPicture`; recipient stores in `tags.picture_token`     |
| Recipient receives token (transitive)                    | Stored in recipient's `tags.picture_token` for `incoming_share` source row                   |
| Picture leaves an OutgoingShare                          | Delete from `share_announcements` → token immediately dead                                   |
| Full share revoked                                       | `DELETE FROM share_announcements WHERE outgoing_share_id = $1` → all tokens dead at once     |

### 5.2 Presign endpoint (modified)

**Old:** `POST /api/federation/pictures/presign` with `{ owner_username, owner_instance, share_token, pictures: [{picture_id, variant}] }`.

**New:** `POST /api/federation/pictures/presign` with `{ pictures: [{picture_token: Uuid, variant: String}] }`.

No federation JWT required (the token is the credential). For each `picture_token`:

```sql
SELECT sa.picture_id
FROM share_announcements sa
WHERE sa.picture_token = $1
```

If not found → 401. If found → presign the picture at the resolved `picture_id`.

This removes `OutgoingShareRepository::has_active_share_for_token` (no longer needed).

### 5.3 Token selection for transitive announcements

When Bob announces picture P (received from Alice) to Carol:

```sql
SELECT t.picture_token
FROM tags t
JOIN incoming_shares is ON t.source_id = is.id
WHERE t.picture_id = $bob_local_picture_id
  AND t.source = 'incoming_share'
  AND is.status = 'active'
ORDER BY t.source_id   -- deterministic; avoids oscillating token selection
LIMIT 1
```

`ORDER BY source_id` ensures the same token is chosen on every run as long as the same `IncomingShare`s are active. When one share is revoked and its
tag row removed, the next run picks the following token in UUID order (deterministic re-selection, triggers exactly one token-refresh re-announce —
see §6.4).

For Bob's own pictures (no `incoming_share` tags): the token comes from Bob's own `share_announcements` row, stable and sender-generated.

---

## 6. Pipeline Extension

### 6.1 New file structure

The current `infra/pipeline.rs` (~258 lines) grows significantly. Split following the project convention (`.rs` file alongside a directory):

```
infra/
  pipeline.rs          # loop infrastructure: create(), run(), sweep()
  pipeline/
    evaluation.rs      # run_for_user() — service evaluation + tag reconciliation
    announcement.rs    # announcement diff step + handle_deleted_pictures()
```

**`infra/pipeline.rs`** keeps:

- `create(db, notify, poll_interval, task_queue, config) → impl Future` (signature extended)
- `run()` — the `tokio::select!` wake/poll loop
- `sweep()` — `find_users_with_dirty_pictures`, iterate, call `evaluation::run_for_user`
- `run_once_for_user(db, task_queue, config, user_id)` — test helper (signature extended)

**`infra/pipeline/evaluation.rs`** gets:

- `run_for_user(db, task_queue, config, user_id) → Result<(), anyhow::Error>` — the full current logic
- After successfully reconciling each batch: calls `announcement::process_batch`

**`infra/pipeline/announcement.rs`** gets:

- `process_batch(db, task_queue, config, user_id, dirty_picture_ids)` — the announcement diff step
- `handle_deleted_pictures(db, task_queue, config, owner_id, deleted_picture_ids)` — called from `cleanup_incoming_share`

`AppState` gains `task_queue` and `config` references already; the pipeline `create()` receives them at startup.

### 6.2 Announcement step (`announcement::process_batch`)

> **Note (initial vs ongoing announce).** The pipeline is the single announce path, with two entry
> points. `process_first_announcements` runs first in `run_for_user` (independent of tagging
> services): for each `pending_first_announcement` `OutgoingShare` — the status a share enters when
> accepted — it announces the current coverage **ignoring `future`** and flips the share to
> `active`, recording tracking rows + status in one transaction. `process_batch` (below) is the
> *ongoing* diff for `active` + `future = true` shares. Both share the per-picture token/tracking
> logic and `AnnounceTaskItem::from_picture`; share acceptance therefore no longer announces
> synchronously — it just sets `pending_first_announcement` and wakes the pipeline.

Called once per batch of successfully reconciled pictures within `evaluation::run_for_user`. Inputs: the user's ID, the list of picture IDs just
reconciled in this batch.

**Step 1 — Load active outgoing shares with `future = true`:**

```sql
SELECT id, tag_path, recipient_username, recipient_instance, owner_id
FROM outgoing_shares
WHERE owner_id = $user_id AND status = 'active' AND future = true
```

If no such shares exist, return immediately.

**Step 2 — Compute current coverage for dirty pictures:**
Which `(picture_id, outgoing_share_id)` pairs are currently active?

```sql
SELECT DISTINCT t.picture_id, os.id AS outgoing_share_id
FROM tags t
JOIN outgoing_shares os
     ON t.tag_path <@ os.tag_path::ltree      -- picture tag is at-or-under share tag
WHERE t.picture_id       = ANY($dirty_ids)
  AND os.owner_id        = $user_id
  AND os.status          = 'active'
  AND os.future          = true
  AND NOT (                                    -- loop prevention
        t.picture_id IN (
            SELECT p.id FROM pictures p
            WHERE p.owner_username = os.recipient_username
              AND p.owner_instance = os.recipient_instance
        )
      )
```

This uses PostgreSQL's ltree `<@` (descendant-of) operator. It also applies loop prevention inline: pictures owned by the share recipient are
excluded.

**Step 3 — Load current tracking entries for dirty pictures:**

```sql
SELECT outgoing_share_id, picture_id, picture_token
FROM share_announcements
WHERE picture_id = ANY($dirty_ids)
  AND outgoing_share_id = ANY($share_ids)
```

**Step 4 — Load current valid tokens for dirty received pictures:**

```sql
SELECT t.picture_id, t.picture_token
FROM tags t
JOIN incoming_shares is ON t.source_id = is.id
WHERE t.picture_id = ANY($dirty_ids)
  AND t.source     = 'incoming_share'
  AND is.status    = 'active'
  AND t.picture_token IS NOT NULL
ORDER BY t.source_id   -- deterministic selection
```

Group by `picture_id`, keep the first token per picture (the `ORDER BY` makes this stable).

**Step 5 — Diff and enqueue:**

For each `(picture_id, share_id)` pair:

| State                                                       | Action                                                                                                                                    |
|-------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| In current coverage, **not** in tracking                    | INSERT into `share_announcements` (generates token), enqueue `AnnounceSharedPictures`                                                     |
| In tracking, **not** in current coverage                    | DELETE from `share_announcements`, enqueue `UnannounceSharedPictures`                                                                     |
| In both, token in tracking **≠** valid token from tags step | UPDATE token in `share_announcements`, enqueue `AnnounceSharedPictures` (re-announce with new token; recipient uses `ON CONFLICT UPDATE`) |
| In both, tokens match                                       | No-op                                                                                                                                     |

Batch announces per `(outgoing_share_id)` to minimise task count.

### 6.3 Token refresh detail

A token mismatch occurs when `cleanup_incoming_share` has removed an `incoming_share` tag row (and its associated token) while another
`incoming_share` tag for the same picture remains. The pipeline detects this on the next run because `cleanup_incoming_share` marks surviving pictures
as dirty (§6.4). The announcement step queries the current valid token (step 4 above), compares with the tracking entry, and re-announces if
different.

**Re-announce uses the same `AnnounceSharedPictures` task.** The recipient's `assign_incoming_share_tag` now uses
`ON CONFLICT DO UPDATE SET picture_token = EXCLUDED.picture_token`, so the new token silently replaces the old one without error.

### 6.4 Dirty-picture triggering for token refresh

`cleanup_incoming_share` currently does not mark surviving pictures as dirty. Add this step after the tag removal and picture deletion:

```rust
// pictures that still have at least one incoming_share tag (survived cleanup)
let survivor_ids = PictureRepository::find_with_any_incoming_share_tag(
    &mut *tx,
    share.recipient_id,
    &all_removed_picture_ids,   // pictures whose incoming_share tag was just removed
).await?;
PipelineRepository::invalidate(&mut *tx, &survivor_ids).await?;
```

Then wake the pipeline: `pipeline_notify.notify_one()`.

This ensures the pipeline re-evaluates surviving pictures and detects any token mismatch in their transitive announcements.

### 6.5 Batch sleep

Add environment variable `PIPELINE_BATCH_SLEEP_MS` (default `0`, documented in `.env.example` and `infra/config.rs`). If non-zero,
`tokio::time::sleep(Duration::from_millis(batch_sleep_ms)).await` is inserted between picture batches in `evaluation::run_for_user`. Provides manual
backpressure for large deployments.

---

## 7. New Flows

### 7.1 Future picture announcement

A picture enters an `OutgoingShare`'s coverage when any of its tags is at or under `outgoing_share.tag_path`. This happens via:

- Ingest (new picture, `last_pipeline_run_at = NULL`)
- Manual tag add
- Tagging service assigns a new tag

All of these already wake the pipeline. The announcement step (§6.2) handles the rest.

**No announcement is sent if `future = false`** on the `OutgoingShare`. The step 1 query filters `future = true`.

### 7.2 Picture unannounce

A picture leaves an `OutgoingShare`'s coverage when:

- Its relevant tag is removed (manual or pipeline removal)
- The picture is deleted (`deleted_at` set, excluded from coverage queries)

For dirty pictures that were processed in this run: the diff step detects lost coverage and enqueues `UnannounceSharedPictures`. The task calls
`POST /api/federation/pictures/unannounce` on the recipient's backend. The recipient removes the `incoming_share` tag for that picture, then deletes
the picture row if no other `incoming_share` tags remain.

For **full share revocation** (see §7.4), individual unannounce is not used — the revocation message is sufficient.

### 7.3 ShareBack

**Initiator (Bob):** calls `POST /api/authenticated/shares/outgoing` with `shareback_of: <Alice's OutgoingShare id>`. The `CreateOutgoingRequest`
already has this field. Bob may specify any tag he owns.

**Sender-side (Bob's backend `create_outgoing_share`):** passes `shareback_of` through unchanged into the `ShareAnnouncement` federation message.

**Recipient-side auto-accept (`receive_share_announcement`):**

```
if shareback_of = Some(original_os_id):
    look up OutgoingShare(original_os_id) on Alice's DB
    verify: owner_id == alice, recipient == bob, allow_share_back == true
    if verified:
        create IncomingShare(status = Active)       ← skip Pending state
        find_or_create_shared_tag_mapping_service(alice)
        add mapping rule: incoming_share_id = new_is.id,
                          assign_tag = original_os.tag_path
        invalidate + wake pipeline
        return auto_accepted = true   ← Bob announces his pictures himself
    else if allow_share_back == false OR share not found:
        create IncomingShare(status = Pending)      ← normal manual-accept flow
        no SharedTagMappingService created
        return auto_accepted = false
```

The auto-accept is **local only** — it does *not* register Bob's pictures or call back to Bob. Instead `shares/announce` returns `auto_accepted`, and
Bob (the initiator) moves his own `OutgoingShare` to `pending_first_announcement` so his pipeline announces his pictures. This keeps the whole
ShareBack with no callback into Bob's uncommitted state (rule 2 of the federation consistency rules in `03_BACKEND_ARCHITECTURE.md`; resolves the
cross-instance ShareBack deadlock).

**`find_or_create_shared_tag_mapping_service(user_id)`:** queries for the first existing `SharedTagMapping`-type service owned by the user. Creates
one if none exists (no `requires`/`excludes`). Returns the service ID. The new mapping rule is inserted via `SharedTagMappingRuleRepository::create`.

**Cross-instance ShareBack flow:** Bob's `create_outgoing_share` calls `POST /api/federation/shares/announce` on Alice's backend (inside Bob's
transaction). Alice auto-accepts locally and returns `auto_accepted = true`. Bob then moves his `OutgoingShare` to `pending_first_announcement` and
commits; **his pipeline** announces his pictures to Alice (`pictures/announce`) and flips the share to `active`. Alice's
`receive_pictures_announcement` registers the pictures (loop prevention: skip if `picture.owner == alice`). **Same-backend ShareBack** does the same:
the local auto-accept runs after commit (non-fatal), then the share moves to `pending_first_announcement` and the pipeline registers the initiator's
pictures for the recipient.

**`allow_share_back` propagation:** `IncomingShareRepository::create` receives and stores `allow_share_back` from the `ShareAnnouncement`.
Same-backend path: read it from the `OutgoingShare` directly.

### 7.4 Transitive revocation

Triggered when `cleanup_incoming_share` runs on Bob's backend for `IncomingShare is-001` (tag path `SharedToMe.alice_AT_...Travel.France`).

After tag removal and picture cleanup, find Bob's outbound shares to auto-revoke:

```sql
SELECT id, recipient_username, recipient_instance
FROM outgoing_shares
WHERE owner_id = $bob_id
  AND status   = 'active'
  AND (
      tag_path::text = $shared_to_me_path
      OR starts_with(tag_path::text, $shared_to_me_path || '.')
  )
```

For each found share: call `revoke_outgoing_share` (which handles same-backend and cross-instance paths, sends the revocation federation message, and
deletes the share's `share_announcements` rows). This cascades the revocation to Carol via the existing revocation flow.

**Scope:** only `SharedToMe.*` tag shares. Mixed-tag shares (e.g. Bob shares `/Photos/Holidays/2024`) are not auto-revoked. The tracking table handles
picture-level cleanup for those via the pipeline's unannounce mechanism.

### 7.5 Loop prevention

**Sender side — in `receive_share_accept` and `accept_incoming_share`:**
After loading pictures from `PictureRepository::list_by_tag_and_owner`, filter out pictures where
`owner_username == recipient_username && owner_instance == recipient_instance`.

**Sender side — in pipeline announcement step (§6.2 step 2):**
The coverage query already excludes pictures where the picture's owner matches the share recipient.
`reconcile_share` additionally skips a share whose recipient resolves to its own owner (a self-share
predating the creation-time rejection) — its announcement would re-dirty the announced rows and wake
the same pipeline forever.

**Recipient side — in `receive_pictures_announcement`:**
Before registering each picture, check:

```rust
if pic.owner_username == config.local_username_of_recipient
   && pic.owner_instance == config.global_domain {
    continue;  // skip: picture owner is the local user
}
```

More precisely: if the recipient's `user_id` can be resolved from `(pic.owner_username, pic.owner_instance)` via `find_local_user_id`, skip the
picture.

---

## 8. Extended `cleanup_incoming_share`

Full revised sequence (single DB transaction where possible):

```
1. TagRepository::remove_incoming_share_tags(tx, share.id)
   → returns Vec<picture_id> of affected pictures

2. PictureRepository::delete_received_without_share_tags(tx, share.recipient_id, sender)
   → returns Vec<Uuid> deleted_picture_ids

3. surviving_ids = affected_picture_ids - deleted_picture_ids

4. For each deleted_picture_id:
     query share_announcements JOIN outgoing_shares for downstream recipients
     group by (outgoing_share_id, recipient_username, recipient_instance)
     for each group: enqueue InternalTask::UnannounceSharedPictures
   then: DELETE FROM share_announcements WHERE picture_id = ANY(deleted_ids)

5. transitive revocation (if this is a full revocation, not tombstone):
     find outgoing_shares WHERE tag_path starts with SharedToMe.<sender>.<tag>
     for each: call revoke_outgoing_share()

6. PipelineRepository::invalidate(tx, surviving_ids)
   (marks surviving pictures dirty for token refresh)

7. SharedTagMappingRuleRepository::flag_broken_for_share(tx, share.id)
   (sets is_broken = true on any SharedTagMappingRule referencing this IncomingShare)

8. IncomingShareRepository::set_status(tx, share.id, final_status)

9. tx.commit()

10. pipeline_notify.notify_one()
11. cache.del(RedisKey::IncomingShareToken(...))  [Redis key being removed with origin_share_token]
```

Step 5 (transitive revocation) only fires if `final_status == Revoked`, not `Tombstoned`. Tombstone is a rejection — there were no pictures to
cascade.

---

## 9. SharedToMe Prefix Protection

`SharedToMe` is a reserved tag prefix. Manual tag creation with this prefix is rejected at all write surfaces. Pipeline service configurations that
would assign a `SharedToMe` tag are also rejected (to prevent pipeline rules from masquerading as incoming shares).

**Validation function** (add to `domain/tag.rs` or `TagPath`):

```rust
pub fn is_reserved_prefix(path: &str) -> bool {
    path == "SharedToMe" || path.starts_with("SharedToMe.")
}
```

**Endpoints that must reject `SharedToMe` values:**

| Endpoint                                                 | Field        |
|----------------------------------------------------------|--------------|
| `PATCH /api/authenticated/tags`                          | `add_tags[]` |
| `POST /api/authenticated/tagging-services/{id}/mappings` | `assign_tag` |
| `POST /api/authenticated/tagging-services/{id}/rules`    | `assign_tag` |
| `POST /api/authenticated/tagging-services/{id}/segments` | `assign_tag` |

Return `400 Bad Request` with message `"SharedToMe is a reserved tag prefix"`.

`requires` and `excludes` fields on tagging services may reference `SharedToMe` (legitimate use: "only run on received pictures"). No restriction
there.

---

## 10. Modified / New Services

### `services/shares.rs`

- `create_outgoing_share`: remove `share_token` from `OutgoingShareRepository::create`; pass `allow_share_back` to same-backend
  `IncomingShareRepository::create`; `shareback_of` is already forwarded to `ShareAnnouncement`.
- `accept_incoming_share` (same-backend path): loop prevention filter on picture list; insert into `share_announcements` for each picture before
  calling `register_received_pictures`; pass `picture_token` per picture.
- `cleanup_incoming_share`: extended as described in §8.
- `revoke_outgoing_share`: after marking share revoked, delete all `share_announcements` rows for this share (
  `ShareAnnouncementRepository::delete_all_for_share`).

### `services/federation.rs`

- `receive_share_announcement`: add `allow_share_back` + `shareback_of` parameters; handle ShareBack auto-accept as described in §7.3; pass
  `allow_share_back` to `IncomingShareRepository::create`.
- `receive_share_accept`: insert into `share_announcements` for each picture before building `AnnouncedPicture`; include `picture_token` in each
  `AnnouncedPicture`; apply loop prevention filter.
- `receive_pictures_announcement`: accept `picture_token` per picture; pass to `register_received_pictures`; apply loop prevention (skip pictures
  owned by local user).
- `receive_pictures_unannouncement` (new): find each picture by `remote_picture_id`, remove its `incoming_share` tag for this share, delete picture
  row if no remaining `incoming_share` tags, mark survivors dirty, wake pipeline.
- `presign_batch_for_token` → replaced by `presign_by_picture_tokens`: takes `Vec<(picture_token, variant)>`, looks up `share_announcements` by token,
  presigns the resolved picture. No JWT required.

### `services/shares.rs` — `register_received_pictures`

Add `picture_token: Uuid` parameter to `ReceivedPictureInfo`. Pass to `TagRepository::assign_incoming_share_tag`.

---

## 11. Modified / New Repositories

### New: `repository/share_announcement.rs`

```rust
ShareAnnouncementRepository::insert(executor, outgoing_share_id, picture_id) -> Uuid
    // INSERT ... RETURNING picture_token

ShareAnnouncementRepository::insert_batch(executor, outgoing_share_id, &[(picture_id)]) -> Vec<(Uuid, Uuid)>
    // returns Vec<(picture_id, picture_token)>

ShareAnnouncementRepository::delete(executor, outgoing_share_id, picture_id)

ShareAnnouncementRepository::delete_for_pictures(executor, &[picture_id])
    // batch delete across all shares — used by cleanup_incoming_share

ShareAnnouncementRepository::delete_all_for_share(executor, outgoing_share_id)
    // used by revoke_outgoing_share

ShareAnnouncementRepository::find_picture_by_token(executor, picture_token) -> Option<Uuid>
    // SELECT picture_id FROM share_announcements WHERE picture_token = $1

ShareAnnouncementRepository::find_announced_for_share(executor, outgoing_share_id) -> Vec<Uuid>
    // all picture_ids currently in tracking for a share

ShareAnnouncementRepository::find_downstream_for_pictures(executor, &[picture_id])
    -> Vec<(Uuid, Uuid, String, String)>  // (outgoing_share_id, picture_id, recipient_username, recipient_instance)
    // used by cleanup_incoming_share to build unannounce tasks

ShareAnnouncementRepository::update_token(executor, outgoing_share_id, picture_id, new_token)
    // UPDATE share_announcements SET picture_token = $3 WHERE ...
    // used by the pipeline token-refresh path
```

### Modified: `repository/share.rs`

- `OutgoingShareRepository::create`: remove `share_token` parameter and column.
- `OutgoingShareRepository::has_active_share_for_token`: **delete** (no longer used).
- `OutgoingShareRepository::list_active_future_by_owner(executor, owner_id) -> Vec<OutgoingShare>`: new — used by pipeline announcement step.
- `OutgoingShareRepository::find_by_shareto_me_prefix(executor, owner_id, ltree_prefix) -> Vec<OutgoingShare>`: new — used by transitive revocation in
  `cleanup_incoming_share`.
- `IncomingShareRepository::create`: add `allow_share_back: bool` parameter.
- `IncomingShareRepository::create`: remove `origin_share_token` parameter.

### Modified: `repository/tag.rs`

- `assign_incoming_share_tag`: add `picture_token: Uuid` parameter; change conflict resolution to
  `ON CONFLICT DO UPDATE SET picture_token = EXCLUDED.picture_token`.
- `remove_incoming_share_tags`: return `Vec<Uuid>` (affected picture IDs) — needed by `cleanup_incoming_share` to compute survivors.

### Modified: `repository/tagging.rs`

- `SharedTagMappingRuleRepository::flag_broken_for_share(executor, incoming_share_id)`: new —
  `UPDATE shared_tag_mapping_rules SET is_broken = true WHERE incoming_share_id = $1`.

### Modified: `repository/pipeline.rs`

- `find_dirty_for_user`: no change.
- `invalidate`: existing, used by `cleanup_incoming_share` for survivors.

### New: `PictureRepository::find_with_any_incoming_share_tag`

```rust
PictureRepository::find_with_any_incoming_share_tag(
    executor, user_id, candidate_picture_ids: &[Uuid]
) -> Vec<Uuid>
// Returns picture IDs from the candidate list that still have
// at least one incoming_share source tag (survivors after cleanup).
```

---

## 12. Modified Federation Protocol

### Modified: `ShareAnnouncement`

Remove field `share_token`. The federation message now carries `allow_share_back` and `shareback_of` only (both already present in the struct).

### Modified: `AnnouncedPicture`

Add field `picture_token: Uuid`.

### New: `PicturesUnannouncement`

```rust
pub struct PicturesUnannouncement {
    pub outgoing_share_id: Uuid,
    pub sender_username:   String,
    pub sender_instance:   String,
    pub picture_ids:       Vec<String>,
}
```

### Modified: `FederationClient` methods

- `announce_share`: remove `share_token` from serialisation.
- `announce_pictures_to_backend`: `PicturesAnnouncement::pictures` now includes `picture_token` per item.
- **New:** `unannounce_pictures_to_backend(sender, recipient, payload: &PicturesUnannouncement)` — `POST /api/federation/pictures/unannounce`.
- `presign_remote_pictures`: signature changes from `(owner, pictures, share_token)` to `(pictures: &[(picture_token, variant)])`. No
  `owner_username`/`owner_instance` needed — the token is self-resolving on Alice's side.

---

## 13. Modified / New API Endpoints

### User-facing

**`POST /api/authenticated/shares/outgoing`**
`CreateOutgoingRequest` already has `shareback_of: Option<Uuid>`. No change to the request shape. Service now handles the ShareBack branch.

**`GET /api/authenticated/shares/outgoing`**
`ShareResponse` add fields: `allow_share_back: bool`, `future: bool`.

**`GET /api/authenticated/shares/incoming`**
`IncomingShareResponse` add fields: `allow_share_back: bool`, `local_mapping_service_id: Option<Uuid>`.

**`PATCH /api/authenticated/tags`**
Validate `add_tags`: reject any path starting with `SharedToMe` with 400.

**`POST /api/authenticated/tagging-services/{id}/mappings`**
**`POST /api/authenticated/tagging-services/{id}/rules`**
**`POST /api/authenticated/tagging-services/{id}/segments`**
Validate `assign_tag`: reject any path starting with `SharedToMe` with 400.

### Federation

**`POST /api/federation/shares/announce`**
Handler passes `allow_share_back` and `shareback_of` to `receive_share_announcement`. `ShareAnnouncement` model loses `share_token`.

**`POST /api/federation/pictures/announce`**
`AnnouncedPicture` model gains `picture_token: Uuid`. Handler passes token to `receive_pictures_announcement` → `register_received_pictures` →
`assign_incoming_share_tag`.

**New: `POST /api/federation/pictures/unannounce`**

```
Auth: Federation JWT (sender → recipient)
Body: PicturesUnannouncement
Handler: federation::receive_pictures_unannouncement()
```

**Modified: `POST /api/federation/pictures/presign`**

Old body: `{ owner_username, owner_instance, share_token, pictures: [{picture_id, variant}] }`.

New body: `{ pictures: [{picture_token: Uuid, variant: String}] }`.

No federation JWT required. Auth is the `picture_token` itself. Handler calls `presign_by_picture_tokens`.

Doc table update in `03_BACKEND_ARCHITECTURE.md`:

```
POST /api/federation/pictures/unannounce  — Unannounce specific pictures from a share.
POST /api/federation/pictures/presign     — Auth: picture_token per picture (no JWT). Body: { pictures: [{picture_token, variant}] }.
```

---

## 14. TaskQueue Extension (`infra/tasks.rs`)

### New task variants

```rust
pub enum InternalTask {
    TagRename { user_id, old_tag, new_tag },  // existing

    /// Announce (or re-announce) pictures to a share recipient.
    /// Used for new coverage and for token refresh (same endpoint, recipient uses ON CONFLICT UPDATE).
    AnnounceSharedPictures {
        outgoing_share_id:    Uuid,
        sender_username:      String,
        recipient_username:   String,
        recipient_instance:   String,
        pictures:             Vec<AnnounceTaskItem>,
        is_same_backend:      bool,
    },

    /// Unannounce specific pictures from a share recipient.
    UnannounceSharedPictures {
        outgoing_share_id:  Uuid,
        sender_username:    String,
        recipient_username: String,
        recipient_instance: String,
        picture_ids:        Vec<String>,   // sender-side local IDs as strings
        is_same_backend:    bool,
    },
}

pub struct AnnounceTaskItem {
    pub picture_id:    Uuid,
    pub picture_token: Uuid,
    pub owner_username: String,
    pub owner_instance: String,
    // metadata fields for announcement payload
    pub filename:      Option<String>,
    pub mime_type:     Option<String>,
    pub file_size:     Option<i64>,
    pub width:         Option<i32>,
    pub height:        Option<i32>,
    pub captured_at:   Option<NaiveDateTime>,
}
```

### `TaskRunner` extended

`TaskRunner` gains `federation: FederationClient` and `config: Config` fields. `create()` signature:

```rust
pub fn create(
    db:         PgPool,
    federation: FederationClient,
    config:     Config,
    concurrency: usize,
) -> (TaskQueue, impl Future<Output = ()>)
```

For **same-backend** announce/unannounce tasks: call the service functions directly (no HTTP). For cross-instance: call
`FederationClient::announce_pictures_to_backend` / `unannounce_pictures_to_backend`.

`AppState` passes `federation` and `config` to `tasks::create()` at startup.

---

## 15. Configuration

Add to `infra/config.rs` and `.env.example`:

```
# Optional: milliseconds to sleep between picture batches in the pipeline (default 0).
# Increase under heavy load to give the database time to breathe.
PIPELINE_BATCH_SLEEP_MS=0
```

---

## 16. Edge Cases

**Same picture covered by two active OutgoingShares to the same recipient.**
Two rows in `share_announcements`, each with a distinct `picture_token`. The recipient has two `incoming_share` tag rows for the same picture, each
with its own token. Both are valid presign credentials. No special handling needed.

**Partial revocation: share A revoked, share B still covers the same picture to the same recipient.**
`cleanup_incoming_share` removes the is-A `incoming_share` tag (and its token). The surviving picture is marked dirty. The pipeline detects a token
mismatch (tracking has token-A, current valid tag has token-B via the `ORDER BY source_id LIMIT 1` query). A re-announce task is enqueued with
token-B. Recipient's `assign_incoming_share_tag` silently updates the stored token via `ON CONFLICT DO UPDATE`. Exactly one re-announce is sent;
subsequent runs find tokens matching and do nothing.

**Bob shares a manual tag that contains Alice's received pictures; Alice partially revokes.**
Bob's tracking table has token-A for picture P. Alice revokes share A. `cleanup_incoming_share` runs on Bob's backend; if Bob's received picture
survives (share B also covers it), the picture is marked dirty. Bob's pipeline runs, detects token-A is no longer in Bob's active `incoming_share`
tags, picks token-B, updates Bob's tracking entry, re-announces to Carol. If the received picture was deleted (share A was the only covering share),
`cleanup_incoming_share` directly queues `UnannounceSharedPictures` to Carol.

**Bob directly re-shares a `SharedToMe.alice.Travel` tag; Alice revokes.**
`cleanup_incoming_share` runs. Transitive revocation (§7.4) finds Bob's outgoing share with that tag prefix and auto-revokes it. Bob's
`revoke_outgoing_share` sends a revocation to Carol and deletes Bob's tracking entries — no per-picture unannounce needed.

**Bob receives same picture via Alice directly AND via a chain through Charlie.**
Both announcements call `PictureRepository::create_received` with `ON CONFLICT DO UPDATE` — one picture row. Two `incoming_share` tag rows, each with
a distinct token. Both are valid. The latest announcement's token wins for the `pictures` table update (not used for presign). Presign uses
`LIMIT 1 ORDER BY source_id` → stable selection.

**ShareBack with `allow_share_back = false`.**
`receive_share_announcement` creates `IncomingShare(status = Pending)`. No auto-accept, no `SharedTagMappingService` rule. Alice must accept manually
via `POST /api/authenticated/shares/incoming/{id}/accept`. No mapping is auto-created.

**`future = false` OutgoingShare.**
The pipeline announcement step (step 1) filters `future = true`. Pictures that existed at accept time were already announced via the share-accept
flow. No future announcements are sent. Revocation still works normally.

**`receive_pictures_announcement` for a `Pending` share.**
Rejected with 404 (existing guard: `if incoming.status != Active → NotFound`). Prevents picture injection into unaccepted shares.

**Pipeline token-refresh query returns different token on each run (non-deterministic).**
Prevented by `ORDER BY source_id`. The same `IncomingShare` row is always selected first (lowest UUID). Token oscillation cannot occur.

---

## 17. Test Scenarios

Tests follow the existing pattern (repository integration tests + service integration tests + federation e2e tests).

### Share announcement / unannounce

- Picture ingested → pipeline runs → picture enters OutgoingShare coverage → announcement task queued → recipient has picture.
- Manual tag removed → picture leaves share coverage → unannounce task queued → recipient no longer has picture.
- `future = false` share: picture added after accept → no announcement sent.

### ShareBack

- Same-backend: Bob creates share with `shareback_of`; Alice's IncomingShare transitions to Active; `SharedTagMappingService` mapping created with
  correct `assign_tag`; Bob's pictures announced to Alice; Alice's pipeline maps them.
- Cross-instance: same flow via federation.
- `allow_share_back = false`: IncomingShare stays Pending; no mapping created.
- Multiple ShareBacks: second ShareBack adds a second mapping rule to the same service.

### Loop prevention

- Alice shares `Photos` to Bob; Bob shares `Photos` back to Alice; Alice's share accept does not re-announce Alice's own pictures to Alice.
- `receive_pictures_announcement`: pictures whose owner matches the local user are silently skipped.

### Transitive sharing (Alice → Bob → Carol)

- Alice shares `Travel` to Bob; Bob re-shares `SharedToMe.alice.Travel` to Carol.
- Alice announces picture P to Bob → Bob announces P to Carol → Carol can presign P from Alice using the forwarded `picture_token`.
- Alice revokes → Bob's received picture deleted → Bob's share to Carol auto-revoked → Carol's IncomingShare set to Revoked.

### Per-picture token security

- Revoke share A; verify `picture_token_A` is no longer valid on Alice's presign endpoint (404/401).
- Share B still active; verify `picture_token_B` still resolves correctly.

### Token refresh (partial revocation)

- Picture covered by shares A and B to Bob; Alice revokes A; Bob's pipeline runs; Bob's tracking entry updated to token-B; Carol's re-announce
  received and token updated.
- No spurious oscillation: subsequent pipeline runs produce no additional tasks for this picture.

### Deduplication

- Alice shares `Travel` (share A) to Bob; Alice also shares `Photos` (share B) to Bob; both cover picture P; Bob has one received picture row, two
  `incoming_share` tags.
- Bob presigns P via either token — both work.

### SharedToMe prefix protection

- `PATCH /api/authenticated/tags` with `add_tags: ["SharedToMe.fake"]` → 400.
- `POST /api/authenticated/tagging-services/{id}/rules` with `assign_tag: "SharedToMe.foo"` → 400.
- Pipeline correctly assigns `SharedToMe.*` tags via `incoming_share` source (not blocked).

### Transitive revocation — mixed tag

- Bob shares `/Photos/Holidays/2024` which includes Alice's received pictures via `SharedTagMappingService`; Alice revokes; Alice's pictures
  unannounced from Carol via tracking table + pipeline; Bob's share to Carol NOT auto-revoked (it still covers Bob's own pictures).
