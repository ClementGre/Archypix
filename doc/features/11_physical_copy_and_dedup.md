# Physical Copy ("Rescue") & Content Dedup

## 1. Overview & goals

A received picture lives on the **owner's** storage; if the owner trashes and purges it, the recipient
loses access ([09 §5](09_trash_and_exif_overrides.md)). This feature lets a user keep it by making a
**physical copy** into their own library, and de-duplicates the identical copies that then exist
across a share graph so the user sees one picture, not many.

1. **Copy endpoint** — duplicate a received (or owned) picture's bytes into the caller's library as a
   new, independent owned picture.
2. **Content dedup** — group byte-identical copies by a metadata-independent `content_hash`; keep one
   **live survivor** and hide the rest as `content_dedupe`, reversibly.
3. **Deleted-content boomerang** — when a copy arrives that matches content the user has **manually**
   deleted, route it straight to trash (recoverable), and keep it there even after the manual twin is
   gone.

**No schema migration of its own** — `pictures.content_hash`, the `copy_source_*` provenance columns,
the `picture_deleted_reason` values `content_dedupe`/`boomerang`, and `idx_pictures_content_hash` are
all already in `001_initial_schema.up.sql` via [09 §4](09_trash_and_exif_overrides.md).

## 2. Decisions (settled)

- **A copy is a new, independent picture** `(copier, new_id)`. **Never reuse the original
  `picture_id`:** `(A, X)` and `(B, X)` are distinct composite ids, so reuse would not dedupe anyway,
  and deduping on bare `picture_id` would break the composite-key identity used by tags, federation,
  and WebDAV. Provenance of the original is recorded in `copy_source_*` (display + survivor
  selection), not as identity.
- **No per-source snapshot/resolver is needed.** The only genuinely divergent sources are
  original-vs-copy, and those are **distinct rows** — each holds its own `remote_exif_data`/lifecycle
  on its own picture row. Same-identity multi-path receipt agrees by the
  [09 invariant](09_trash_and_exif_overrides.md), so it never diverges. Dedup is therefore a
  *display* concern handled by soft-deleting redundancy, not by merging rows.
- **Dedup soft-deletes the redundant copies; it never merges identities.** Keep one live survivor per
  `content_hash` group; soft-delete the rest as `content_dedupe`. This reuses trash's view-exclusion
  for free — WebDAV and every default view just show non-deleted rows, so the survivor *is* the
  representative (no read-side selection logic).
- **`content_hash` excludes metadata** so it is stable across EXIF edits and changes only on visual
  edits (§4). Not blurhash (too coarse), not decoded-pixel (decoder drift across instances).
- **The dedup reconciler runs serial-per-user in the pipeline** so it cannot race into two-survivor /
  zero-survivor states.
- **`content_dedupe` is relational and reversible; `boomerang` is a sticky content rejection** (§5.3).

## 3. Copy endpoint

`POST /api/authenticated/pictures/{id}/copy` — copy a received (or owned) picture into the caller's
library.

- Creates a **new owned picture row** (`local_user_id = caller`, `remote_picture_id = NULL`,
  `owner_* = NULL`) with `copy_source_owner_username/instance/picture_id` set to the source picture's
  **owner identity** (for a received source: its `owner_*`; for a transitive copy: the source's
  `copy_source_*` root, so provenance points at the genuine original, not the intermediary).
- Copies the **bytes** server-side (source `pictures` object → staging → caller's `pictures` object),
  reusing the upload pipeline; enqueues `gen_thumbnail`, which computes `content_hash`, `file_hash`,
  thumbnails, EXIF (existing job — extended to emit `content_hash`, §4).
- The copied row's EXIF is seeded from the source's **effective** `exif_data` at copy time (a copy is
  a snapshot — it does not stay linked to the owner; subsequent owner edits do not flow into it).
- Cross-instance source: fetch bytes via the existing per-picture presign
  ([01_better_sharing_support.md](01_better_sharing_support.md)); the owner must be reachable.

Once owned, the copy behaves like any owned picture (shareable, editable, versioned).

## 4. `content_hash`

- **Definition:** a hash of the image **excluding metadata segments** (the entropy-coded scan data;
  e.g. strip JPEG APPn/EXIF/XMP, PNG text chunks). Format-specific stripping for the common formats;
  fall back to `NULL` (then dedup falls back to `file_hash`) for formats we can't strip.
- **Computed by the worker** in `gen_thumbnail` (it already decodes/parses the file) and stored on the
  picture row; **forwarded downstream** in announcements (`AnnouncedPicture` gains `content_hash`), so
  recipients can group across owners.
- **Deterministic across instances** (it is just bytes — no decoder in the loop), which matters
  because A's, B's and C's copies are hashed by *different* backends.
- **Stable across EXIF edits, changes on visual edits.** EXIF edits are metadata-only rewrites
  (gexiv2 leaves the scan data untouched) → same `content_hash`; a visual re-encode changes the scan
  data → new `content_hash`. So the key is "updated only on visual edits" **without classifying the
  edit** — it is computed from the result, so even a blind WebDAV PUT is handled the same way.
  *(Requires `edit_picture` to keep doing metadata-only rewrites for EXIF-only edits — which it does,
  and which is correct regardless, since re-encoding on every EXIF change would be lossy.)*

## 5. The dedup reconciler

Runs in the pipeline (serial per user). For each `content_hash` group of a user's rows it maintains
**exactly one live survivor**, with the rest `content_dedupe`-deleted. Group rows fall in four roles:
**live**, **`manual`-deleted**, **`content_dedupe`-deleted**, **`boomerang`-deleted**.

### 5.1 Survivor selection

Deterministic, recomputed whenever the group changes: prefer **not-owner-deleted**, then
**owned-by-me** (a durable local copy beats a soon-to-vanish received original), then the
**original owner** (per `copy_source_*` / `owner_*`), then **lowest id**. The chosen row is live; the
other live rows are `content_dedupe`-deleted.

### 5.2 Re-evaluation triggers

New copy arrives; a row's `content_hash` changes (re-announced visual edit); a row is removed (owner
purge / unannounce); a manual delete or restore. On a `content_hash` change a row **leaves its old
group** (re-evaluate that group for promotion) and is **classified fresh** in the new group.

### 5.3 The promotion rule (this is the `content_dedupe` ⁄ `boomerang` distinction)

> **Promote a `content_dedupe` row to live iff its group has no live row *and* no row deleted with
> reason `manual` or `boomerang`.**

- **`content_dedupe` is relational.** It exists only to avoid showing a duplicate. When the survivor
  disappears via a *system* event (owner purge / unannounce removes the row), the group becomes purely
  `content_dedupe` → promote the best one. **This is rescue-on-purge for free:** D already holds B's
  copy hidden; when A's original purges, B's surfaces.
- **A `manual`-deleted row blocks promotion.** If the user manually trashed the content, its
  `content_dedupe` siblings stay hidden — the user rejected this content, not merely a row.
- **`boomerang` is a sticky content rejection.** A `boomerang` row is **never** promoted by group
  state, and its presence **blocks** promotion of siblings. It leaves the boomerang state **only when
  its own `content_hash` changes** (a visual edit makes it genuinely different content), at which
  point it exits the group and is reclassified. So a boomerang copy stays trashed **even after the
  manual twin that triggered it is gone** (the owned twin purged after retention, or the received twin
  unannounced) — the rejection outlives the twin.

### 5.4 Classifying an arrival

For a newly-received (or newly-copied) row, by its `content_hash` group's state:

| Group state                                 | Arrival becomes                                                            |
|---------------------------------------------|----------------------------------------------------------------------------|
| empty (no matching rows)                    | **live**                                                                   |
| has a live survivor                         | **`content_dedupe`**                                                       |
| no live, only `content_dedupe` rows         | **live** (it is the rescue/promotion case)                                 |
| has a `manual` or `boomerang` row (no live) | **`boomerang`**, flagged "you previously deleted this; shared by *sender*" |

### 5.5 Trash representative, restore & a stable survivor (implemented refinements)

A content group is **Live** (no rejection) or **Rejected** (≥1 `manual`/`boomerang` row). Beyond the
base rules above, the implementation maintains:

- **One representative per state.** Hidden rows (`content_dedupe`/`boomerang`) never appear in any
  list — `push_filters` shows live + `manual` only. So a Live group shows its one live survivor and a
  Rejected group shows its **one `manual` trash representative**, never a pile of duplicates.
- **Delete rejects the whole content group, by priority.** A manual delete (`reject_content_group`)
  trashes every copy: the **best** one (priority §5.1 — prefer not-owner-deleted, then **owned/local**,
  then original, then lowest id) becomes the single `manual` representative, the rest `boomerang`. The
  priority is applied **at delete time** (not just by the reconciler), so the local/owned copy is
  deleted first and shown as the representative immediately — the trash never shows a received
  "owner's copy untouched" entry while a local copy hides as a boomerang — and the next reconcile,
  which picks the same `best()`, never replaces it. When the representative later disappears
  (purge/unannounce/permanent-delete) the best `boomerang` is promoted to `manual` so the rejected
  content still shows one recoverable trash entry.
- **Restore lifts the rejection:** restoring the manual representative flips its `boomerang` siblings
  back to `content_dedupe`, so a later disappearance of the restored row rescue-promotes a copy
  instead of leaving only boomerangs.
- **A stable survivor.** The reconciler never reshuffles a correct single-live group — survivor
  selection only *collapses* a transient multi-live group or *promotes* when none is live. So whichever
  copy is live stays live, and a user can pick one without a "pinned" column.

> **Future — permanent delete.** When the permanent-delete feature lands, emptying a rejected group's
> `manual` representative should **also permanently delete its `boomerang` siblings** (they are copies
> of content the user rejected). The reconciler already promotes a `boomerang` to `manual` when the
> representative disappears, so until that feature exists a rejected group always keeps one visible
> trash entry; but a permanent delete that removes only the `manual` row would leave `boomerang` rows
> (and, if any is a *local* owned file, real bytes) behind — permanent delete must sweep the whole
> group.

### 5.6 Consumers

- **WebDAV and default views** show only the live (non-deleted) survivor / the manual trash
  representative — no special grouping logic.
- **`GET /pictures/{id}/copies`** returns the whole group (survivor + hidden siblings) with both
  hashes (so the client distinguishes "same image, EXIF-only difference" from "different content"),
  each row's dedup state, last-edit time, and owner/provenance. The frontend renders this as a
  foldable "Copies" section under the picture details.
- **`POST /pictures/{id}/copies/keep`** makes a chosen copy the live survivor (hiding the others);
  the stable reconciler makes the choice stick.

## 6. Boomerang vs. owner-match loop prevention

Existing owner-match loop prevention ([01 §6.6](../01_GENERAL_SPECIFICATIONS.md)) suppresses only
**relays of the recipient's own pictures** (owner = recipient). A physical copy **launders the owner
identity** (owner = the copier), so it slips through — that is exactly the gap boomerang closes, at
**receive** time. Deliberately **not**:

- **silent discard** — a heuristic content match must not lose a share; auto-trash is recoverable;
- **source-side suppression of copies** — would permanently block the recipient from ever
  re-receiving content that originated from them; auto-trash leaves restore open;
- **deleted-content tombstones** — the guard only needs to cover the grace window. Once the original
  is purged the recipient has no record of it, and a fresh copy is acceptable (re-trashable). The
  boomerang stickiness (§5.3) covers the lingering-copy case without a tombstone table.

## 7. Edge cases

1. **Copy of a copy (transitive).** `copy_source_*` points at the provenance **root** (the genuine
   original), so survivor selection and "from A/B/C" provenance stay correct across chains.
2. **Survivor visually edited** → its `content_hash` changes → it leaves the group; remaining
   `content_dedupe` rows (now purely `content_dedupe`) promote one. Both old and new content are shown
   (correctly — they are now different images).
3. **Owner of the original re-announces a metadata edit** while a copy exists → only the original's
   row updates; the copy is independent (snapshot at copy time, §3). `content_hash` unchanged (EXIF
   edit) → grouping unaffected.
4. **Two copies made concurrently** (B and C of the same A picture) arriving at D → both join the
   group; the reconciler keeps one survivor, `content_dedupe`-hides the other; serial-per-user
   execution prevents a two-survivor race.
5. **User restores a `boomerang` row manually** → treat as a manual restore: clear
   `deleted_at`/`deleted_reason`; it re-enters its group and the reconciler may immediately
   `content_dedupe` it again if a live survivor exists, or keep it live otherwise. (Explicit user
   action overrides the rejection.)
6. **Non-strippable format** (`content_hash IS NULL`) → fall back to `file_hash` for grouping; exact
   bytes still dedupe, EXIF-edit stability is lost for that format only.
7. **Storage/quota:** a copy consumes the copier's storage (it is a real owned file); relevant once
   quotas land.

## 8. WebDAV write-back on a collapsed group (open question)

When a hierarchy maps several copies into the **same** directory, only the survivor is shown, so a
WebDAV move/delete naturally targets the survivor's row. **Undecided:** whether deleting the
representative should affect the hidden `content_dedupe` siblings (delete-the-content) or only the
survivor (after which the reconciler promotes a sibling, resurrecting the file). The natural
`/SharedToMe/<sender>/…` layout sidesteps this (copies sit in per-sender folders); it only bites under
a deliberate same-directory mapping. Resolve when WebDAV directory-level ops land
([99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md) "Advanced WebDav").

## 9. Documentation updates

- [01_GENERAL_SPECIFICATIONS.md](../01_GENERAL_SPECIFICATIONS.md) — copy as a distinct identity;
  `content_hash` dedup; boomerang vs owner-match loop prevention (§6.6).
- [02_INFRASTRUCTURE_DESIGN.md](../02_INFRASTRUCTURE_DESIGN.md) / worker docs — `gen_thumbnail` emits
  `content_hash`; `AnnouncedPicture` carries it.
- [03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md) — copy endpoint; dedup reconciler in the
  pipeline; `deleted_reason` lifecycle.
- [06_API_REFERENCE.md](../06_API_REFERENCE.md) — `POST /pictures/{id}/copy`.

## 10. Work breakdown

- [x] Worker: compute metadata-stripped `content_hash` in `gen_thumbnail` (`imaging/content_hash.rs`);
  report it via `CompleteJobRequest`; backend stores it (`update_from_worker`/`update_after_processing`);
  `AnnouncedPicture` + recipient write path (`create_received`) carry/persist it. `edit_picture`
  recomputes it from the result so a visual edit regroups.
- [x] Copy endpoint `POST /pictures/{id}/copy` (`services::pictures::copy_picture`,
  `PictureRepository::create_copy`): byte copy (same-backend S3 copy / cross-instance presign+fetch),
  new owned row, `copy_source_*` provenance (root-resolved), `gen_thumbnail` enqueue (`is_initial = false`
  so the seeded effective EXIF is kept).
- [x] Dedup reconciler in the pipeline (`infra::pipeline::dedup`, serial per user): survivor selection
  (§5.1), rescue-promotion (§5.3), arrival classification (§5.4); `content_hash`-change regrouping via
  the recompute on edit. Candidate-key + sweep queries in `repository::dedup`.
- [x] Boomerang guard at the announce-receive path (`classify_arrival` in `register_received_pictures`,
  §5.4 / §6) + the user-clarified manual-delete→boomerang of `content_dedupe` siblings (trash paths);
  flag + recoverable trash.
- [x] Frontend: copy/"rescue" action in the selection panel (incl. a prominent button in the
  owner-deleting grace banner) and the lightbox header; copy-of provenance line; a foldable **Copies**
  section (`CopiesSection`) under the picture details listing the whole content group with state /
  hashes-diff / owner / last-edit and a "Keep this" control; an admin **Thumbnail & content-hash
  regeneration** panel in the Jobs tab.
- [x] Copies/keep + admin regen endpoints: `GET /pictures/{id}/copies`,
  `POST /pictures/{id}/copies/keep`, `POST /admin/pictures/regenerate-thumbnails`; the trash view
  hides `content_dedupe`/`boomerang`; manual-delete/restore lifecycle triggers; a stable reconciler
  (no pin column needed).
- [x] Tests (`back/tests/physical_copy_dedup.rs` + worker `content_hash` unit tests): copy creates a
  distinct owned identity + provenance root; dedup keeps one survivor; rescue-on-purge promotes a
  sibling; manual delete boomerangs siblings and blocks rescue; arrival into a rejected group
  boomerangs; arrival with a live survivor is content_dedupe'd; content_hash stable across metadata,
  changes on scan/framing.
- [x] Docs (§9). §8 (WebDAV directory-op write-back on a collapsed group) remains an open question,
  revisited when WebDAV directory ops land.
