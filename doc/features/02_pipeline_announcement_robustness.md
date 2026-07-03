# Pipeline announcement robustness & per-user wake model

Status: **implemented**. §7 (the `mpsc<Uuid>` per-user waker + concurrent scheduler) landed first;
§3–§6 and §8 (the `errored` state, the unified inline reconcile, deliver-then-record, deleted-picture
handling, the `is_same_backend` resolver fix, and per-share backoff) landed in a follow-up. See §10.

This note refines the pipeline-driven announcement model introduced in
`01_better_sharing_support.md`. It fixes three correctness problems and one ergonomics problem in
the announcement path, and restructures the pipeline loop for per-user isolation.

## 1. Problems with the current path

1. **Same-backend mis-routing.** `is_same_backend` is computed as
   `share.recipient_instance == config.global_domain`. That is only the *necessary* condition — two
   backends can share one global domain, so a recipient on the same global domain may still live on
   a *different* backend. The correct test is whether the recipient *user* resolves locally, exactly
   what `services::users::find_local_user_id` already encodes (it returns `None` both when the
   instance differs and when the user is not in this backend's DB). The hand-rolled comparison
   appears three times in `infra/pipeline/announcement.rs` and once in `services/shares/lifecycle.rs`.

2. **Announcement consistency hole.** Today the pipeline commits the `share_announcements` tracking
   rows and flips the `OutgoingShare` to `Active`, then enqueues an *ephemeral* delivery task. If
   that task fails (or the process restarts — the in-process `TaskQueue` does **not** survive
   restarts and is allowed to fail), the tracking row persists and claims "announced", so the next
   sweep's diff treats the picture as delivered and never retries. The picture is silently lost to
   that recipient while the share looks healthy.

3. **Deleted pictures drop out of the share.** `find_dirty_for_user`, `find_users_with_dirty_pictures`,
   `current_coverage`, and `list_by_tag_for_user` all filter `deleted_at IS NULL`. A soft-deleted
   picture therefore leaves the coverage set, the diff sees "tracked but not covered", and the
   recipient is spuriously *unannounced* — even though the blob still exists and the user may want to
   review shared-but-trashed pictures during the retention window.

4. **No retry/backoff story.** Because retry rode on the ephemeral task, there was no durable,
   restart-safe mechanism to re-attempt a failed delivery, and no throttle against a down recipient.

## 2. Principles

- **The `share_announcements` tracking table is the single source of truth for what has actually
  been delivered.** A row exists **iff** the recipient has been told about that `(share, picture)`.
- **Dirtiness is only an optimization.** It bounds the *Active* fast-path to the pictures that
  changed. Correctness never depends on it: any full reconcile re-derives the same result from
  coverage-vs-tracking.
- **Deliver, then record.** The durable state (tracking rows, status flips) is written *after* a
  successful delivery, never before. A failed delivery leaves no tracking row, so the next
  reconcile re-attempts it. This makes the reconciliation loop itself the retry engine — no outbox
  table, no durable queue.
- **The `TaskQueue` is ephemeral and allowed to fail.** It is no longer the system of record for
  announcement delivery; delivery moves *inline* into the per-user pipeline run so its success or
  failure can drive share status.
- **Both paths use deliver-then-record.** Same-backend "delivery" is the recipient-side
  `register_received_pictures` (its own transaction); cross-instance is the federation HTTP call.
  Both run *before* the tracking rows are written, and both are idempotent (the recipient upserts
  pictures/tags last-writer-wins), so a partial failure self-heals on the next reconcile. (An earlier
  draft kept same-backend fully transactional with the tracking insert; in practice the idempotent
  register + reconcile loop make that unnecessary, and it avoids threading a transaction through
  `register_received_pictures`.)

## 3. Share state machine

The `share_status` enum (shared by `OutgoingShare` and `IncomingShare`) gains `Errored`:

```
Pending | PendingFirstAnnouncement | Active | Errored | Revoked | Tombstoned
```

`PendingFirstAnnouncement` and `Errored` are the same logic — *full coverage diffed against the
tracking table* — and differ only by entry reason (and the UI label: "never delivered" vs "delivery
degraded"). `Active` is the same diff with coverage scoped to dirty pictures.

| Status (outgoing)          | Reconcile coverage scope   | On full success | On failure                      |
|----------------------------|----------------------------|-----------------|---------------------------------|
| `PendingFirstAnnouncement` | all pictures under the tag | → `Active`      | stay `PendingFirstAnnouncement` |
| `Errored`                  | all pictures under the tag | → `Active`      | stay `Errored` (+ backoff)      |
| `Active` (+ `future=true`) | dirty pictures only        | stay `Active`   | → `Errored` (+ backoff)         |

Notes:

- **Only `Active → Errored`.** A failure during `PendingFirstAnnouncement` simply stays there. A
  `future=false` share never reaches the incremental path, so it can only ever go
  `PendingFirstAnnouncement → Active` and is never demoted.
- `Errored` is **per-share**: one degraded recipient does not pull the user's other shares off the
  Active fast path.
- The demotion is the recovery bridge: a failed incremental announce flips the share to `Errored`,
  which forces a *full* coverage scan next pass, so the missed picture (and any new coverage that
  arrived meanwhile) is recovered even though the picture is no longer dirty.

### One reconcile, two scopes

`process_first_announcements` and `process_batch` collapse into a single `reconcile_share(share,
scope)` where `scope ∈ { Full, Dirty(ids) }`. Both: compute coverage, diff against tracking,
announce the untracked, unannounce the orphaned, and — because both diff against the existing
tracking rows — never double-announce. `PendingFirstAnnouncement` just happens to start with empty
tracking.

## 4. Deliver-then-record ordering

For the cross-instance path:

```
mint token (in memory)
  → build announce payload carrying that token
  → federation call (NO transaction open)
  → on success: BEGIN; insert tracking rows; flip status; COMMIT
  → on failure: set Errored (+ next_retry_at); leave tracking untouched
```

The federation HTTP call is **not** inside a transaction — holding a pooled connection and row
locks across a cross-instance call would let one slow/unreachable recipient drain the pool and
block unrelated queries (the classic "I/O inside a transaction" trap, which hurts most on a bulk
background loop).

This is safe even in the "call succeeded but our commit/process died" window because the token
machinery reconverges:

- The reconcile **skips owned pictures that already have a tracking row** (re-announce only fires for
  new coverage or a received picture whose upstream token moved), so an owned picture's token is
  stable once recorded.
- A re-announce after a lost commit re-mints a fresh `Uuid` and writes it with
  `insert_with_token` (`ON CONFLICT DO UPDATE SET picture_token = EXCLUDED.picture_token`); the
  recipient's `register_received_pictures` is likewise last-writer-wins, so it adopts the new token
  and the sender's tracking table (the authority) and the recipient agree again.

Tokens are therefore minted client-side (`Uuid::new_v4()` → payload → `insert_with_token` on
success) — the same pattern already used for forwarded/received tokens.

Unannounce is symmetric: call first, delete the tracking rows on success; on failure set `Errored`
and keep the rows for retry.

## 5. Same-backend routing fix

Replace every `is_same_backend = recipient_instance == global_domain` with a resolver call:

```rust
let recipient_local = find_local_user_id(cache, db, config, recipient_username, recipient_instance).await?;
// same-backend  ⇔ recipient_local.is_some()
```

Both branches use the deliver-then-record ordering of §4 (same-backend "delivers" via
`register_received_pictures`); the resolver result only decides *which* delivery mechanism runs.
This fix is load-bearing: consistency semantics — not just routing — branch on it, and the same hand
-rolled comparison was also fixed in `cleanup_incoming_share`'s downstream-revocation cascade.

## 6. Deleted pictures stay announced until permanent deletion

Soft-delete must change nothing about a share; only **permanent** deletion unannounces.

- **Drop `deleted_at IS NULL` from the dirty and coverage queries together**: `find_dirty_for_user`,
  `find_users_with_dirty_pictures`, and the per-share coverage query (`coverage_for_share`, which
  replaced the old `current_coverage` / `list_by_tag_for_user`). They must move as a set — if the
  dirty queries include soft-deleted but coverage does not, the diff spuriously unannounces every
  soft-deleted picture (the bug we are fixing). Deleted pictures are still tagged by the pipeline
  (re-derived once, then they leave the dirty set).
- **Permanent deletion owns its unannounce** (+ tracking cleanup). Once the `pictures` row is gone,
  reconciliation cannot derive it, so the retention job that hard-deletes must explicitly unannounce
  and delete the tracking rows first.
- **Recipient-visible "deleted" is out of scope here.** Keeping the picture announced lets the
  recipient keep viewing it during the retention window with *no* payload change. Propagating the
  owner's `deleted_at` onto the recipient's copy (spec §2) needs a `deleted_at` field on the announce
  payload and on `register_received_pictures`; that belongs with the **Trash & restore** roadmap item.

## 7. Per-user wake model: `mpsc<Uuid>` + concurrent scheduler  *(implemented)*

### Motivation

The previous loop used a single global `tokio::sync::Notify` ("something changed somewhere") and a
sequential `for user in dirty_users { run_for_user().await }` sweep. Once delivery moves inline,
that sequential sweep means one unreachable recipient stalls *every* user behind it. We want
per-user isolation and bounded concurrency, and we already know *which* user each event concerns.

### `PipelineWaker`

> **Superseded by feature 17.** `PipelineWaker` is now `RoutineHandle<Uuid>` and the per-user
> scheduler below is the generic `infra::routine` runtime; `wake`/`wake_debounced` → `trigger`/
> `trigger_debounced`, `channel`+`create` → `routine::spawn`. The semantics described here are
> unchanged — see `doc/features/17_unified_routine_framework.md`.

`AppState.pipeline_notify: Arc<Notify>` is replaced by `AppState.pipeline_waker: PipelineWaker`, a
cheap clone wrapping `mpsc::UnboundedSender<Uuid>`:

```rust
#[derive(Clone)]
pub struct PipelineWaker { tx: mpsc::UnboundedSender<Uuid> }

impl PipelineWaker {
    pub fn wake(&self, user_id: Uuid) { let _ = self.tx.send(user_id); }
    pub fn disconnected() -> Self { /* tests: rx dropped, wakes discarded */ }
}

/// Build the waker + the receiver consumed by the loop. Lets `main` wire the waker into the
/// TaskQueue (which wakes recipients) before the loop future is built — breaking the
/// waker ↔ task_queue cycle.
pub fn channel() -> (PipelineWaker, mpsc::UnboundedReceiver<Uuid>);
```

Every producer calls `pipeline_waker.wake(target_user_id)`. A *missed* wake is only a latency bug,
never a correctness bug: the poll-interval recovery sweep re-enqueues all dirty users (and all
`PendingFirstAnnouncement`/`Errored` share owners) regardless.

### Wake targets

The target is always the user whose pictures or shares changed — **not necessarily the request
caller**:

| Producer                                          | Wake target                      |
|---------------------------------------------------|----------------------------------|
| `complete_upload`, tag edit, tagging-service CRUD | the authenticated owner          |
| `receive_share_accept`                            | `share.owner_id` (the sender)    |
| `accept_incoming_share` (same-backend)            | the resolved sender's local id   |
| `auto_accept_shareback_local`                     | the local recipient who accepted |
| same-backend `deliver_announce/unannounce`        | `incoming.recipient_id`          |
| `receive_pictures_announcement` (registered > 0)  | `incoming.recipient_id`          |
| `receive_pictures_unannouncement`                 | `incoming.recipient_id`          |
| `cleanup_incoming_share`                          | `share.recipient_id`             |

### Scheduler

The loop owns the receiver and a per-user coalescing scheduler with bounded concurrency:

```rust
enum RunState { Running, Rerun }          // per user_id
state: Arc<Mutex<HashMap<Uuid, RunState>>>
sem:   Arc<Semaphore>                      // PIPELINE_CONCURRENCY permits
```

- **schedule(user_id)** (sync): if the user is absent from the map, insert `Running` and spawn a
  worker; if already present, set `Rerun` and return (coalesce — a wake that arrives mid-run is not
  lost).
- **worker**: acquire a semaphore permit, run `run_for_user`, release the permit, then under the
  lock: if state is `Rerun` reset to `Running` and loop again (events arrived during the run); else
  remove the user and exit. The `std::sync::Mutex` is only ever held across synchronous code, never
  across `.await`.
- **loop**: `select!` between `rx.recv()` (→ schedule) and `sleep(poll_interval)` (→ schedule every
  user from `find_users_with_dirty_pictures`, the restart/lost-wake recovery). One recovery sweep
  also runs at startup.

This gives concurrency *across* users (bounded by `PIPELINE_CONCURRENCY`, default 4) and strict
serialization *per* user (one worker per user_id at a time — concurrent runs for the same user would
race on its tag reconcile and tracking writes; different users touch disjoint rows). A persistently
failing `run_for_user` does not hot-loop: the worker exits after one failure and is only retried by
the next external wake or the poll sweep.

### `main` wiring

```rust
let (pipeline_waker, pipeline_rx) = infra::pipeline::channel();
let (task_queue, task_runner) = tasks::create(.., pipeline_waker.clone(), ..); // runner wakes recipients
tokio::spawn(task_runner);
// The pipeline delivers inline, so it holds the federation client (cross-instance announce), the
// cache (same-backend resolution via find_local_user_id) and the waker (wake same-backend recipients):
tokio::spawn(infra::pipeline::create(db, pipeline_rx, poll, config, concurrency, federation, cache, pipeline_waker.clone()));
// AppState holds task_queue + pipeline_waker
```

## 8. Backoff

A down recipient leaves a share in `Errored`/`PendingFirstAnnouncement`, which the wake query
re-enqueues on every event and every poll — and each attempt is now a *full* coverage scan plus an
HTTP call. To throttle, add two columns to `outgoing_shares`:

- `last_error_at TIMESTAMP NULL`
- `next_retry_at TIMESTAMP NULL`

set on a failed delivery to `now + PIPELINE_RETRY_BACKOFF_SECS` (a fixed backoff for the MVP; an
attempt-count-driven exponential schedule can be layered on later), and filter them both in the
recovery branch of `find_users_with_dirty_pictures` and in `list_announceable_by_owner`:

```sql
... WHERE status IN ('pending_first_announcement','errored')
      AND (next_retry_at IS NULL OR next_retry_at <= now())
```

Because the retry signal lives on the **share**, not the picture, backoff is a single timestamp
check rather than per-picture state.

## 9. Migrations

Per `03_BACKEND_ARCHITECTURE.md` §I "Database migrations", schema changes are edited **directly** into
`001_initial_schema.up.sql` (no `ALTER`, no new migration files):

- add `errored` to the `share_status` enum definition;
- add `last_error_at` / `next_retry_at` to `outgoing_shares`;
- then `cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare`.

## 10. Implementation status

All implemented:

- [x] §7 — `PipelineWaker` (`mpsc<Uuid>`), per-user coalescing scheduler, bounded concurrency,
  poll/startup recovery sweep, `PIPELINE_CONCURRENCY` config, all wake producers retargeted to the
  affected user.
- [x] §5 — `is_same_backend` resolver fix via `find_local_user_id` (in the pipeline reconcile and in
  `cleanup_incoming_share`'s downstream cascade).
- [x] §3/§4 — `errored` status; unified inline reconcile (`reconcile_pending_and_errored` +
  `reconcile_active_batch`, both over an internal `reconcile_share`); deliver-then-record ordering;
  `delivery.rs` and the `AnnounceSharedPictures` task variant removed (the pipeline announces
  inline; only the best-effort revocation-cascade `UnannounceSharedPictures` task remains).
- [x] §6 — soft-deleted pictures stay tagged and announced (`deleted_at` filter dropped from the
  dirty + coverage queries). Permanent-deletion-owned unannounce is still pending the **Trash &
  restore** roadmap item (no hard-delete path exists yet).
- [x] §8 — `last_error_at` / `next_retry_at` on `outgoing_shares`, `PIPELINE_RETRY_BACKOFF_SECS`
  config, and the backoff filter in `find_users_with_dirty_pictures` + `list_announceable_by_owner`.

Follow-ups (out of scope here): recipient-visible deleted propagation and permanent-deletion
unannounce (both land with Trash & restore); an attempt-count-driven exponential backoff schedule.
