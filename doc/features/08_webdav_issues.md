# WebDAV issues & follow-ups

Two issues surfaced while exercising the WebDAV server (`doc/features/06_webdav.md`) from
real OS file clients. **Issue A** (atomic-save writes) is **implemented** (spec below;
`services/vfs.rs` + `api/webdav.rs`, tests in `back/tests/vfs.rs`). **Issue B** (custom
directory names on mirror nodes) is **deferred** — see §2 for why.

---

## 1. Issue A — atomic-save ("safe-save") writes

### 1.1 Symptom

Editing `Onewheel/le_mien_de_Onewheel/phare.jpg` in macOS **Preview** does not overwrite the
file in place. Preview (like every `NSDocument` app, and the Windows/Office/Linux equivalents)
performs an **atomic save**: it writes the new bytes to a scratch sibling, then renames the
scratch over the original so a crash can never leave a half-written file. Over WebDAV this
became the following sequence (trimmed log at the end of this section):

1. `MKCOL Onewheel/…/phare.jpg.sb-93035015-3rqb93` — a temp **directory** sibling of the
   target; name = `<original>.sb-<8hex>-<6alnum>`.
2. `PUT …/phare.jpg.sb-…/phare.jpg` — an empty placeholder, then the real edited bytes,
   written **inside** that temp directory.
3. It *intends to* rename the scratch file over the original `phare.jpg`, then delete the
   temp directory.

Two behaviours of the current server break this:

- **The mirror auto-tag treats `.sb-…` as a real folder** (§9 of 06_webdav). The inner PUT
  mints a garbage tag (`…phare_jpg_sb_93035015_3rqb93`) and **ingests a spurious duplicate
  picture** (`vfs put: ingest new picture` in the log).
- **The slug transform renames the temp directory**, so Preview's follow-up PROPFIND on the
  *exact* name it created returns `404`. Preview can't find its own scratch dir, retries
  twice, and gives up — **the rename-over-original never happens and the edit is lost.**

### 1.2 Goal

An atomic save of an existing picture must land as a plain **overwrite** of that picture
(new bytes, versioned per the user's `versioning_mode`), with **no** new tag, **no** duplicate
picture, and **no** phantom directory. An atomic save that creates a *new* file must land as a
normal new-picture ingest at the final path. The mechanism must be **client-agnostic** —
macOS, Windows, Linux, rclone — and **crash-safe**: an interrupted save never touches the
original.

### 1.3 Model — an atomic-staging namespace

The fix generalises the existing junk-sidecar idea (§11) and pending-dir idea (§9) into a
third transient class: **atomic-staging**. A path recognised as an atomic-write scratch
artifact (§1.4) is *never* resolved through the tag tree, never mints a tag, and never ingests
a picture on its own. Its bytes live only in a transient staging area (S3 staging bucket +
Redis marker, §1.7) until a **terminal rename** promotes them (§1.5–1.6) or a TTL GC discards
them (§1.8).

The key invariant that makes this robust: **the authoritative target is the MOVE
destination, not the temp name.** The recognizer only decides "defer, don't ingest"; the
subsequent rename tells us the real picture to overwrite (or the real path to create). So even
an opaque random temp name works, as long as the client issues the rename — which is the whole
point of an atomic save.

### 1.4 Recognizers

A curated, extensible detector `is_atomic_staging(name)` (sibling to today's `is_ignored`).
Two shapes; the list lives as a small constant in the WebDAV layer and is easy to extend:

| Platform / tool                | Scratch pattern                                               | Notes                                     |
|--------------------------------|---------------------------------------------------------------|-------------------------------------------|
| macOS `NSDocument`             | `<base>.sb-<8hex>-<6alnum>` (dir **or** file)                 | The Preview case. Temp is a *directory*.  |
| Windows `ReplaceFile`/Explorer | `<base>.tmp`, `<base>~`, random `<8hex>.tmp`                  | Temp is a file, renamed over the target.  |
| Linux / rsync / editors        | `.<base>.<rand>`, `<base>.part`, `<base>.partial`, `#<base>#` | Hidden or suffixed scratch.               |
| Generic / opaque               | random name with no recoverable base                          | Handled purely by the terminal MOVE dest. |

Recognizer precedence: **junk (§11) → atomic-staging (§1) → normal tag-tree resolution.**
A name matches at most one class. OS *metadata/lock* files (`.DS_Store`, `._*`, `~$<name>`,
`.~lock.<name>#`, `Thumbs.db`, …) stay **junk sidecars** (Redis-inline, ≤1 MiB, never a
picture) — they carry no picture bytes. Atomic-staging is only for scratch artifacts that
carry real, picture-destined bytes.

> The recognizer list is conservative but not load-bearing for correctness: a name that is
> genuinely wanted but happens to match (e.g. a real file called `report.tmp`) would simply
> never persist unless a MOVE promotes it — and such names are never image content. The cost of
> a false positive is bounded by the TTL GC.

### 1.5 Operation handling

For any request whose target (or an ancestor directory) is an atomic-staging path, dispatch to
the staging handler **before** tag-tree resolution:

- **MKCOL** (macOS temp dir) → record a Redis **staging-dir** marker (analogous to the
  pending-dir marker, but flagged so it never mints a tag). `201`. Round-trips in PROPFIND
  under its **exact** name (no slugification) so the client finds it.
- **PUT** (empty body) → accepted no-op placeholder (as today), nothing staged.
- **PUT** (real bytes) → stream to a temp file and hash inline (existing `stream_to_temp`),
  upload the bytes to the **staging bucket**, and record a marker
  `(hierarchy, staging-path) → { staging_key, hash, size, content_type, mtime }` (TTL). `201`.
  **No** picture, **no** tag.
- **GET / HEAD / PROPFIND** of a staging path → serve back from the marker (exact name,
  correct size/ETag; bytes via staging-bucket redirect or proxy). Some apps read back what
  they wrote to verify — this must succeed.
- **MOVE — the commit.** Three cases by source/destination:
    - `MOVE <staging-path> → <real path>` → **promote** the staged bytes (§1.6), then drop the
      marker + staging object. *This is the step that was previously lost.*
    - `MOVE <real picture> → <staging-path>` (the "move the original out to a backup" step of an
      exchange) → record the source `picture_id` in the marker, mutate nothing. A later `DELETE`
      of that staging path discards the reference; the original is untouched.
    - `MOVE <staging-path> → <staging-path>` → just relocate the marker.
- **COPY** with a staging endpoint → analogous but additive: `COPY staging → real` promotes as
  a new/overwrite **without** removing the source; other COPY forms relocate/duplicate the
  marker. (Rare in practice.)
- **DELETE** of a staging path or dir → drop the marker **and** delete the staging-bucket
  object. No picture touched.

### 1.6 Promotion = overwrite or new

Promotion reuses the existing `put_file` finalize logic (`back/src/services/vfs.rs`), the only
difference being the **byte source is an S3 staging object, not a local temp file**. Refactor
`put_file`'s tail into a shared core `finalize_write(dest_segments, byte_source, hash, size,
content_type)` where `byte_source ∈ { LocalTemp(path), Staging(key) }`; a direct PUT passes
`LocalTemp`, a promote passes `Staging`. The core keeps the current decision tree unchanged:

- Destination path resolves to an **existing picture** → **overwrite**: snapshot per
  `versioning_mode` (`snapshot_version_on_overwrite`), copy staging→pictures
  (`Storage::copy_object`, server-side — no re-stream), `set_file_hash`, enqueue
  `gen_thumbnail` keyed on the new hash. Identical bytes (hash match) → no-op.
- Destination path is **new** → run the dedupe chain (§8 of 06_webdav): live hash match →
  retag; trashed hash match → un-delete + retag; else ingest a genuine new picture from the
  staging object and apply the target `onAdd` / mirror auto-tag.

Because the hash is already computed at PUT time, promotion needs no re-read of the bytes; the
server-side S3 copy from staging→pictures (and staging→versions for the snapshot) is the only
data movement.

### 1.7 Staging storage & markers

- **Bytes:** the existing `s3_bucket_staging` bucket, key
  `webdav-staging/{hierarchy_id}/{uuid}`. This is what "use the staging bucket" means — real
  picture bytes (multi-MB) never sit in Redis.
- **Marker:** a new `RedisKey::WebdavStaging(hierarchy_id, path)` →
  `webdav:staging:{h}:{path}`, value the JSON in §1.5, TTL `TRANSIENT_TTL_SECS` (the same
  day-long TTL as pending-dirs/sidecars). Staging-dir markers reuse the pending-dir shape with
  an `atomic: true` flag so they are never turned into tags.

### 1.8 Crash safety (no bespoke GC)

If the client dies between the PUT and the terminal MOVE, nothing is left to clean up
explicitly: the Redis marker expires by its TTL, and the orphaned staging object ages out via
the **staging bucket's existing lifecycle/retention rule** — the same mechanism the webapp
upload path already relies on (it stages to this bucket and never runs its own GC). No new
sweep routine is introduced. **Default is always safe:** no promotion, no mutation, no data
loss; the original picture was never touched.

### 1.9 Interaction with existing behaviour

- **§9 mirror auto-tag** is bypassed for staging paths — that is the bug fix. Only the
  *promoted* destination path runs auto-tag, and only when it is a genuine new file under a
  mirror.
- **§11 junk sidecars** are unchanged and take precedence (§1.4). Two disjoint transient
  classes.
- **Overwrite tag-writability:** promotion-as-overwrite rewrites bytes only, so (like a direct
  overwrite-PUT today) it is allowed even in a directory that is read-only for *tag* mutation.
  Received/shared pictures remain non-overwritable (`Forbidden`).

### 1.10 Edge cases

- **Atomic save of a new file** (not an overwrite) → the terminal MOVE lands on a
  non-existent path → normal new-picture ingest (+ mirror auto-tag if applicable).
- **Exchange-style save** (`MOVE original→backup`, `MOVE new→original`, `DELETE backup`) →
  covered by the three MOVE cases in §1.5; the backup ref is discarded on DELETE.
- **Windows 50 MB Basic-auth cap** (§12 of 06_webdav) is unchanged — a documented client
  limitation, not worked around here.
- **Two concurrent atomic saves of the same file** → each stages under its own unique temp
  name; the last terminal MOVE wins, snapshotting the prior via `versioning_mode`. No
  corruption (S3 copy is atomic at the object level).

### 1.11 Code touch-points

- `back/src/api/webdav.rs` — `is_atomic_staging` recognizer; dispatch staging paths before
  tag-tree resolution; MKCOL/PUT/GET/PROPFIND/MOVE/COPY/DELETE staging branches.
- `back/src/services/vfs.rs` — staging markers (get/set/clear, mirroring the pending-dir/
  sidecar helpers); `finalize_write` refactor of `put_file`'s tail with a `byte_source` enum;
  `promote_staging(from, to)` called from `move_`/`copy`.
- `back/src/infra/redis.rs` — `RedisKey::WebdavStaging`.
- `back/src/infra/s3.rs` — reuse `copy_object` (staging→pictures / staging→versions),
  `put_object_file`, `delete_object`; no new helper expected.
- No schema change; no new picture/tag columns.

### 1.12 Testing

- **Recognizer** unit tests: macOS `.sb-…` (dir + file), Windows `.tmp`/`~`, rsync
  `.<base>.<rand>`; junk vs staging precedence; opaque temp.
- **VFS end-to-end** (extend `back/tests/vfs.rs`): the full macOS Preview sequence
  (MKCOL → empty PUT → real PUT → PROPFIND round-trip → MOVE-over-original) resolves to a
  **single overwrite** with a version snapshot and **no** new tag/picture; the Windows
  temp-file→MOVE variant; atomic save of a **new** file → new picture; crash-before-MOVE →
  TTL GC leaves the original untouched; `MOVE original→backup`+`DELETE backup` is a no-op.

### 1.13 Documentation to update (on implementation)

- **06_webdav.md** — add an "atomic-save / staging namespace" subsection (a fourth transient
  class alongside pending-dirs §9 and junk sidecars §11) and note it in the write taxonomy
  (§7.1 MOVE row) and §21 implementation status.
- **99_ROADMAP_MVP.md** and this file's work-breakdown — tick when shipped.

### Reference log (trimmed)

```
PROPFIND path=…/phare.jpg.sb-93035015-oDc68c            → 404
PROPFIND path=…/phare.jpg.sb-93035015-3rqb93            → 404
MKCOL    path=…/phare.jpg.sb-93035015-3rqb93            → recorded pending mirror sub-directory  ← should be atomic-staging dir
PROPFIND path=…/phare.jpg.sb-…-3rqb93/phare.jpg         → 404
PUT      path=…/phare.jpg.sb-…-3rqb93/phare.jpg         → empty body, accepted without ingesting
LOCK/UNLOCK …
PUT      path=…/phare.jpg.sb-…-3rqb93/phare.jpg         → ingest new picture (2032838 bytes)     ← should stage, not ingest
         apply onAdd assigns=["…phare_jpg_sb_93035015_3rqb93"]                                    ← spurious tag
PROPFIND path=…/phare.jpg.sb-…-3rqb93/phare.jpg         → 404 (name was slugified; client lost)  ← breaks the rename-over
```

### 1.14 Work breakdown

- [x] `is_atomic_staging` recognizer + precedence over §11/§9.
- [x] `RedisKey::WebdavStaging` + staging marker helpers in `services/vfs.rs`.
- [x] Staging-bucket PUT/GET/DELETE for scratch bytes; staging-dir MKCOL marker.
- [x] `finalize_write` refactor (`ByteSource` enum) + `promote_staging` on MOVE/COPY.
- [x] Wire MOVE commit cases (staging→real / real→staging / staging→staging).
- [x] Recognizer + VFS end-to-end tests (§1.12).
- [x] Doc updates (§1.13): 06_webdav.md §2/§7.1/§13/§21.

---

## 2. Issue B — custom directory names on mirror nodes (DEFERRED)

**Deferred; not to be implemented for now.**

### 2.1 The issue

When a directory is created under a `mirror` node, the folder keeps its typed name, but when a
picture is uploaded the directory name is **slugified** into the tag (no spaces, no accents,
`[A-Za-z0-9_]` only). So the tag differs from the folder name, and on the next mount the folder
**displays** as the slug. Two consequences:

1. **Custom names are lost** — `Mes Photos de Noël` shows back as `Mes_Photos_de_Noel`.
2. **Large uploads re-download** — a sync client sees the folder it uploaded into "rename"
   itself and re-fetches the whole subtree.

### 2.2 Why deferred

Every fix trades away something core:

- **Broaden the ltree charset** — can't represent spaces, dots, slashes, or case-insensitive
  filesystems anyway, and touches every tag / predicate / rule / hierarchy-config / ltree
  query (large blast radius) for a partial fix.
- **name↔tag map in the mirror node config** — introduces per-folder mutable state that must
  be maintained on every rename/move/delete, and is only a hierarchy-local alias.
- **A human title on the tag (separate from its ltree label)** — the cleanest model and it
  would also help the webapp, but it **breaks the simplicity of tags** (a tag is currently just
  its ltree path) and adds schema + rename-interaction complexity.

All three add instability or complexity out of proportion to the benefit right now. The
current behaviour (slugified tag as the folder name) stands; users choosing tag-friendly
folder names avoid the surprise. Revisit if/when a tag "title" concept is wanted for other
reasons.
