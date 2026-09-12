# Feature 32: `file_modified_at` — an honest WebDAV last-modified

## 1. Overview & goals

WebDAV listings report `<D:getlastmodified>` from `pictures.updated_at`
(`services/vfs.rs::project_files`). `updated_at` is maintained by a blanket `BEFORE UPDATE`
trigger, so **any** row write moves it — including the pipeline's
`UPDATE pictures SET last_pipeline_run_at = NULL` on every re-tag.

Sync clients that compare **mtime + size** (rclone's default, Finder, most mobile file apps)
therefore see every tagged picture as changed and re-download it. Tagging a few thousand pictures
can re-transfer most of a library.

Goal: report a last-modified that moves **only when the bytes move**, without weakening
`updated_at`, which is load-bearing elsewhere (`announced_updated_at` gates federation metadata
re-announce, feature 28 §7; the picture list exposes an `updated_at` sort).

Out of scope: directory mtimes / CTag (see 99_ROADMAP_MVP "Advanced WebDav"), and seeding
`file_modified_at` from `original_file_created_at` so an imported library keeps its original dates.

## 2. Decisions (settled)

- **A new column, not a narrowed `updated_at`.** `pictures.file_modified_at`, `NOT NULL`,
  default `now()`. `updated_at` keeps its "anything changed" meaning.
- **Derived from `file_hash`, by trigger.** A dedicated `BEFORE UPDATE ... WHEN (NEW.file_hash IS
  DISTINCT FROM OLD.file_hash)` trigger stamps it. `file_hash` is the SHA-256 of the stored object
  and is already the WebDAV ETag, so this makes the two change signals **agree by construction**:
  nothing can change the ETag without moving the mtime, and nothing else can move it.
    - A per-call-site `SET file_modified_at = now()` was rejected: there are six byte-writing paths
      today (upload, WebDAV overwrite, worker extraction, worker EXIF write-back, copy, announcement
      refresh) and adding a seventh must not be able to silently regress the property.
    - The blanket `updated_at` trigger is the *opposite* pattern (fires on every column); the `WHEN`
      clause is what makes a trigger the right tool here.
- **Identical bytes never bump it.** An idempotent re-PUT, a worker retry, or a re-extraction
  reports the same hash → `IS DISTINCT FROM` is false → no stamp, no re-download.
- **Received pictures are stamped locally, and `remote_updated_at` is left alone.** The
  announcement upsert refreshes `file_hash`, so a recipient's mtime moves when the owner's bytes
  change — timed at *"when this instance learned of it"*, not the owner's clock. `AnnouncedPicture`
  carries **no** `file_modified_at`:
    - `remote_updated_at` is a *version*, not an mtime. It gates the stale-announcement guard (feature 28 §7) and must keep tracking the owner's
      `updated_at`: an owner-side re-tag is a
      legitimate announcement (share coverage, EXIF, creator and deletion state ride along) and must
      be applied. Gating on a bytes-only timestamp would silently drop those.
    - Announcing the owner's `file_modified_at` would only change the *value* of the recipient's
      timestamp, not its behaviour — the local hash-derived stamp already moves exactly when the
      owner's bytes move. It would cost an `AnnouncedPicture` field and so a message `VERSION` bump (lockstep peer upgrade, feature 28 §5), and would
      import the owner's clock skew into the
      recipient's mtimes. Not worth it.
- **Backfill = `updated_at`.** Minimises one-off churn: for a picture not re-tagged since a client's
  last sync, `updated_at` is exactly the value that client already recorded.

## 3. Why an old mtime on a newly-appearing path is fine

Tagging a picture into a new hierarchy directory makes it appear at a new WebDAV path with its **old** mtime. That is correct and safe:

- A client downloads a path it does not have locally *unconditionally*; mtime/size/ETag only
  decide *changed or not* for paths present on both sides. Nothing infers "older than my last sync,
  therefore I must already have it" — that inference would break every restore-from-backup.
- An old date on a fresh path is what `cp -p`, a restore, or `rclone copy` of an old tree produces.
- rclone explicitly supports it (`--use-server-modtime`; it sets the local mtime from the server).

Two consequences that are *not* about the date, for the record:

- A re-tag that **moves** a picture between directories still looks like delete-at-old-path +
  create-at-new-path, so the file is re-fetched unless the client does rename detection (`rclone --track-renames`, which matches on size + hash —
  stable here, another reason mtime and
  ETag must agree).
- Bidirectional sync (`rclone bisync`, davfs2 with local edits) compares both ways; freezing the
  server mtime removes phantom server-side newness, so it strictly improves.

## 4. Schema changes

Migration `0014_file_modified_at` (03 §I — never edit an applied migration):

```sql
ALTER TABLE pictures
    ADD COLUMN file_modified_at timestamp without time zone
        NOT NULL DEFAULT (now() AT TIME ZONE 'utc');

UPDATE pictures SET file_modified_at = updated_at;

CREATE OR REPLACE FUNCTION update_file_modified_at_column() RETURNS TRIGGER AS $$
BEGIN
    NEW.file_modified_at = (now() at time zone 'utc');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_pictures_file_modified_at
    BEFORE UPDATE ON pictures
    FOR EACH ROW
    WHEN (NEW.file_hash IS DISTINCT FROM OLD.file_hash)
EXECUTE FUNCTION update_file_modified_at_column();
```

Both `BEFORE UPDATE` row triggers fire in name order (`…file_modified_at` before `…updated_at`) and
touch disjoint columns, so they do not interact.

## 5. What moves it, and what does not

| Event                                                                 | `file_hash`   | `file_modified_at`           |
|-----------------------------------------------------------------------|---------------|------------------------------|
| Upload / WebDAV PUT of new content, copy                              | set on insert | insert default (= now)       |
| First `gen_thumbnail` completion (hash first computed)                | NULL → hash   | bumps (seconds after ingest) |
| WebDAV overwrite PUT with different bytes                             | changes       | bumps                        |
| WebDAV overwrite PUT with identical bytes (idempotent re-PUT)         | unchanged     | **unchanged**                |
| Worker EXIF write-back / `edit_picture` (file rewritten)              | changes       | bumps                        |
| Re-extraction of an unchanged file, worker retry                      | unchanged     | **unchanged**                |
| Tagging, pipeline run, rule re-evaluation                             | unchanged     | **unchanged**                |
| Rename (MOVE), trash / restore, creator or override edits             | unchanged     | **unchanged**                |
| Announcement refresh of a received picture (owner re-wrote)           | changes       | bumps                        |
| Re-announce after an owner-side **re-tag** (newer `owner_updated_at`) | unchanged     | **unchanged**                |
| Dedup reconciler (hides a duplicate row; no byte swap)                | unchanged     | **unchanged**                |

An EXIF edit that writes through to the file *does* legitimately change the bytes and so bumps both
mtime and ETag — a client re-download there is correct, not a regression.

## 6. Code changes

- `domain::picture::Picture` — `pub file_modified_at: NaiveDateTime`, last field (the `query_as!`
  macro maps columns **positionally**; every `SELECT`/`RETURNING` list appends it last).
- `repository::picture` — the column added to all nine `Picture`-returning queries plus the
  `QueryBuilder` list. No new setter: the trigger owns the value.
- `services::vfs::project_files` — `modified: p.file_modified_at` instead of `p.updated_at`.

No API surface: the field is not serialised into any DTO, so 06_API_REFERENCE is unchanged.

## 7. Testing

`back/tests/vfs.rs`:

- `tagging_does_not_move_webdav_last_modified` — the regression this feature exists for: seed,
  read the listing's `modified`, re-tag (and run the pipeline `UPDATE`), re-list → unchanged while
  `updated_at` has moved.
- `overwrite_put_moves_last_modified` — a different-bytes PUT bumps it; `modified >` the old value
  and the ETag changed with it.
- `identical_reput_does_not_move_last_modified` — hash-match no-op leaves it alone.
- `exif_write_back_moves_file_modified_at_only_when_the_hash_changes` (`worker_contract.rs`) — an
  `edit_picture` completion reporting a new `file_hash` bumps it; one reporting the same hash does not.
- `reannounce_moves_last_modified_only_when_the_owner_bytes_change` (`services_shares.rs`) — an
  owner-side re-tag re-announces with a newer `owner_updated_at` and the same hash: the guard's
  version advances, the recipient's mtime does not; a new owner hash does move it.

## 8. Implementation status

- [x] Migration `0014_file_modified_at` (column + backfill + hash-keyed trigger).
- [x] `Picture.file_modified_at` threaded through every repository read.
- [x] `project_files` serves it as `getlastmodified`.
- [x] Tests: tagging no-move, overwrite move, identical-re-PUT no-move, MOVE/trash no-move,
  worker EXIF write move, received re-announce no-move.
