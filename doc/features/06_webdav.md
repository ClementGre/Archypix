# WebDAV

## 1. Overview & goals

Roadmap item **"WebDAV"** ([99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md)): a bidirectional
filesystem over the **hierarchy** resolver. A user mounts a hierarchy as a network drive
(macOS Finder, Windows Explorer, Linux GVFS/davfs2, rclone, Cyberduck, mobile file apps).
Reads render the hierarchy's directory tree and serve picture bytes; writes
(PUT/MOVE/COPY/DELETE/MKCOL) translate filesystem operations back into tag mutations and
uploads.

This builds directly on [`05_hierarchies.md`](05_hierarchies.md), which was designed
write-ready: the `config` already declares the write-back model (op-lists, compliance,
`safeDeleteMode`, `naming`), the read resolver already produces a per-directory
`TagPredicate`, and `pictures.file_hash` is already the WebDAV ETag. **WebDAV adds no new
columns to the picture/tag model** — only three fields on `hierarchies` (§14).

Core invariant inherited from hierarchies: **a hierarchy stores no pictures.** Every
directory is a live function of the tag graph. WebDAV is just a second front-end over the
same resolver, exposed through a protocol-agnostic `VirtualFs` (§5) so SFTP or another
adapter can be added later without touching the resolver.

Scope of this spec:

- The **auth** model (per-hierarchy encrypted token, HTTP Basic, slug mount).
- The **server**: `dav-server` crate on Axum, locking, body limits.
- The **`VirtualFs`** abstraction and how the resolver feeds it.
- **Reads** (presigned redirect or backend proxy) and **writes** (the full operation
  taxonomy, streaming-to-S3 with inline hashing, identity resolution, mirror auto-tag).
- **Case-sensitivity**, **dotfiles**, and **sync-client** behaviour.
- **Caching**, **DB/config changes**, **module layout**, **API**, **testing**.

## 2. Decisions (settled)

- **Protocol = WebDAV, behind a `VirtualFs` trait (§5).** WebDAV is the only protocol with
  native OS drive-mounting. The resolver is exposed through a protocol-agnostic trait so a
  future SFTP adapter reuses it.
- **Auth = one encrypted token per hierarchy (§3).** A single high-entropy token per
  hierarchy, **encrypted at rest** with an HKDF-derived sub-key of `JWT_SECRET` (no new env
  var), viewable and regeneratable by the owner, gated on `hierarchies.enabled`. Carried as
  the HTTP **Basic password**; validated token→`(user, hierarchy, versioning_mode)` is
  cached in Redis.
- **Mount = `/webdav/{slug}` (§4).** One mount per hierarchy. The path segment is the
  **slugified hierarchy name** (human-readable, so the user recognises the link); the token
  alone identifies the hierarchy and user, and the slug is verified against it. Renaming a
  hierarchy changes its mount URL (expected).
- **Server = `dav-server` crate, class 2 (§4).** In-memory lock store (no Redis). Axum
  dispatches WebDAV methods via an `any` route. Body limit raised only on write routes
  (reads redirect or stream, never buffer).
- **Reads = presigned redirect, proxy fallback (§6).** `GET` → `302` to the presigned
  original; a per-hierarchy `webdav_use_redirect` boolean (default `true`) switches to a
  backend byte-proxy for clients that don't follow redirects. Cross-instance received
  pictures redirect/proxy through the owner's federation presign.
- **Writes = backend-proxied, never presigned (§7).** A `PUT` body streams through the
  backend to the staging bucket while SHA-256 is computed inline; then the existing
  upload-complete transaction runs and enqueues `gen_thumbnail` (EXIF + thumbnails +
  blurhash + hash). Overwrite creates a new `picture_version` **iff** the user's
  `versioning_mode` allows it.
- **Identity (§8).** Explicit ops (overwrite-PUT, MOVE, COPY, DELETE) resolve the target
  picture by **path** (via the naming strategy). A new-content PUT dedupes by **inline
  hash** against the user's owned pictures — a hash hit is a relocate (retag, no new
  picture), including **un-deleting** a recently-trashed match (covers naive
  delete+upload renames under `fullDelete`). ETag = `file_hash`.
- **Mirror auto-tag (§9).** A write into a path whose nearest existing ancestor is a
  `mirror` node assigns the **deepest** tag derived from the path segments
  (`tagRoot + new segments`). `MKCOL` is rejected on static/query nodes; under a mirror it
  is **sidecar**ed (transient Redis dir) so a follow-up file can mint the tag. An `onRemove`
  that would have to drop a **non-manual** tag (a live service still asserts it) **rejects**
  the write (`409`).
- **Case-sensitivity (§10).** (a) Authored sibling-name uniqueness is **case-insensitive**
  (reject at save). (b) Mirror-expanded case-only sibling tags are **surfaced + tolerated**
  (webapp warning; WebDAV exposes both). (c) On write, **case-insensitive sibling match**
  reuses an existing-cased tag; a case-colliding new tag is rejected (`409`).
- **Dotfiles (§11).** `.DS_Store`, `._*`, `Thumbs.db`, etc. are **sidecar**ed in Redis
  (accepted so clients don't hang) and never become pictures.
- **Caching (§13).** The expanded directory tree per hierarchy is cached in Redis (the
  webapp caches client-side; WebDAV must cache server-side).

## 3. Authentication

### 3.1 Token

Each hierarchy owns **one** WebDAV token: a 32-byte random value, hex-encoded (64 chars),
generated on demand. WebDAV clients speak only HTTP **Basic** (and Digest), so the token
rides as the Basic **password**; the username field is the user's `@user` (or anything —
the token is self-identifying). The token is the sole bearer of authority.

### 3.2 Storage — encrypted at rest

The token must be **displayable at any time** (the owner reads it from the API to paste
into a client), so it cannot be a one-way hash. It is stored **encrypted** with
**AES-256-GCM**:

- Key derivation: a **SHA-256 domain-separated sub-key** —
  `SHA256(b"archypix-webdav-token-enc-v1" ‖ jwt_secret)` → a dedicated 32-byte key (the
  label is prepended so the digest output is not a plain hash of the secret). This satisfies
  key separation (the JWT HMAC key is never used directly as a cipher key), avoids a new env
  var, and avoids pulling a second `sha2`/digest version for HKDF. The `-v1` label allows
  rotation later.
- Per-token random 96-bit nonce. Columns: `webdav_token_enc BYTEA` (nonce ‖ ciphertext ‖
  tag) — a single opaque blob; `NULL` ⇒ no token issued yet.
- Helpers live in `infra/crypto.rs`: `webdav_cipher(jwt_secret) -> Aes256Gcm`,
  `encrypt_token` / `decrypt_token`.

A DB leak alone does not expose tokens; an attacker also needs `JWT_SECRET`. The tokens are
low-stakes capabilities (scoped to one of the user's own hierarchy views) — encrypt-at-rest
is the right balance for the "view anytime" requirement.

### 3.3 Validation & cache

On each WebDAV request the Basic password is looked up:

1. Redis `webdav:token:{sha256(token)}` → `{ hierarchy_id, owner_id, versioning_mode,
   use_redirect, enabled }` (the cache key is the hash so the plaintext token is never a
   Redis key). Short TTL (e.g. 300 s).
2. On miss: scan is impossible (token is encrypted, not hashed). Instead the **slug** in the
   path narrows the candidate set — look up the owner's hierarchies whose name slugifies to
   that segment, decrypt each `webdav_token_enc`, and constant-time-compare. (In practice
   the slug + owner is unique; the candidate set is ~1.) Cache the result.

Because the slug alone doesn't carry the owner, the **username** in Basic auth supplies it
(`@user` → `owner_id`); the token then confirms the specific hierarchy. Auth fails closed
when `enabled = false`.

The cache entry is invalidated on token regeneration, hierarchy rename/disable, and settings
change (it carries `versioning_mode`/`use_redirect`).

### 3.4 API surface

Token management is a normal authenticated (User JWT) endpoint on the hierarchy (§17):
`GET …/webdav` returns the current token + mount URL; `POST …/webdav/regenerate` rotates it;
`PATCH …/webdav` toggles `use_redirect`.

## 4. Server, routing & locking

- **Crate:** [`dav-server`](https://crates.io/crates/dav-server) (maintained successor to
  `webdav-handler`). It implements the protocol surface (PROPFIND/PROPPATCH/OPTIONS/LOCK XML,
  Depth handling, method parsing); we implement its `DavFileSystem` over the `VirtualFs`
  (§5) plus a lock store. Hand-rolling PROPFIND XML is explicitly avoided.
- **Axum integration:** WebDAV methods (PROPFIND, MKCOL, COPY, MOVE, LOCK, UNLOCK) are not
  in axum's `MethodFilter`. Mount the dav handler under
  `Router::new().route("/webdav/{slug}", any(handler)).route("/webdav/{slug}/{*path}", any(handler))`
  and let `dav-server` dispatch on the raw method. Auth (§3) runs in a thin layer **before**
  delegating to `dav-server`.
- **Locking:** Finder requires WebDAV **class 2** (LOCK/UNLOCK). Use `dav-server`'s
  in-memory `MemLs` lock store (per-process; sufficient — locks are short-lived advisory).
- **Body limit:** raise/disable `DefaultBodyLimit` only on the WebDAV write routes and
  **stream** the body (§7); never buffer. Reads (§6) carry no request body. Add a
  `WEBDAV_MAX_UPLOAD_BYTES` config (default generous, e.g. 5 GiB) as a guard.
- **`Depth: infinity`** PROPFIND is rejected with `403` (RFC-permitted) to avoid resolving
  the whole tree.

## 5. The `VirtualFs` abstraction

The resolver (`services::hierarchy`) is the single source of truth for both front-ends. To
keep WebDAV from leaking into it, the WebDAV layer consumes a small protocol-agnostic trait
(the `DavFileSystem` impl is a thin adapter over it):

```rust
// services::hierarchy (or a new services::vfs)
pub struct VfsEntry {
    pub name: String,          // file or directory segment name (naming strategy applied)
    pub is_dir: bool,
    pub size: u64,             // pictures.file_size (0 for dirs)
    pub modified: DateTime<Utc>,
    pub etag: Option<String>,  // pictures.file_hash for files
    pub picture_id: Option<Uuid>,
    pub writable: bool,        // from the §7.5 writability matrix
}

#[async_trait]
pub trait VirtualFs {
    async fn list_dir(&self, ctx: &VfsCtx, path: &VfsPath) -> Result<Vec<VfsEntry>, AppError>;
    async fn stat(&self, ctx: &VfsCtx, path: &VfsPath) -> Result<VfsEntry, AppError>;
    async fn read(&self, ctx: &VfsCtx, path: &VfsPath) -> Result<ReadTarget, AppError>;     // §6
    async fn write(&self, ctx: &VfsCtx, path: &VfsPath, body: BodyStream) -> Result<(), AppError>; // §7
    async fn delete(&self, ctx: &VfsCtx, path: &VfsPath) -> Result<(), AppError>;
    async fn rename(&self, ctx: &VfsCtx, from: &VfsPath, to: &VfsPath) -> Result<(), AppError>; // MOVE
    async fn copy(&self, ctx: &VfsCtx, from: &VfsPath, to: &VfsPath) -> Result<(), AppError>;
    async fn mkdir(&self, ctx: &VfsCtx, path: &VfsPath) -> Result<(), AppError>;             // §9
}

pub enum ReadTarget { Redirect(String), Proxy(BodyStream) }  // §6
```

`VfsCtx` carries `owner_id`, the loaded `HierarchyConfig`, and the cached expansion. The
implementation reuses `build_tree` / `predicate_for_path` (read) and the write-back op-lists
(write). `list_dir`/`stat` map directly onto the existing `tree` resolution plus a per-file
projection driven by the directory's `naming` strategy and collision rules (§8 of the
hierarchies spec).

## 6. Reads

`GET`/`HEAD` on a file resolve `path` → `picture_id` (§8) and return the **original**
variant (the real photo; thumbnails are a webapp concern):

- **`webdav_use_redirect = true` (default):** respond `302 Found` with the presigned S3 URL
  (owner-local) or the cross-instance federation presign URL (received pictures). rclone,
  davfs2/neon, Cyberduck follow it.
- **`webdav_use_redirect = false`:** the backend streams the bytes (proxy mode) — fetch from
  S3 (or the owner's presign) and pipe to the response. For clients (some Finder/Windows
  builds) that don't follow `302` on a WebDAV GET. Supports HTTP `Range` for seek.

`ReadTarget::Redirect`/`Proxy` (§5) encodes the choice. Directory `GET` is not meaningful
(PROPFIND lists). The proxy path needs a streaming S3 `get_object` — add
`Storage::get_object_stream` (§16).

## 7. Writes

A WebDAV `PUT` is a single request carrying the whole body, so it **cannot** be presigned
(a `307` redirect would force clients to re-send large bodies, which most don't). The
backend receives the body and **streams it to the staging bucket via S3 multipart while
computing SHA-256 inline**, then runs the existing upload-complete transaction. Inline
hashing gives identity (§8) and a stable ETag immediately, before the worker runs.

Any write that creates or replaces bytes enqueues **`gen_thumbnail`** (EXIF extraction,
thumbnails, blurhash, final hash) — identical to the existing upload path.

### 7.1 Operation taxonomy

| Method                    | Meaning                                    | Action                                                                                                                              |
|---------------------------|--------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------|
| `PUT` (new path)          | new picture, or a relocate as fresh upload | stream→staging + inline hash; dedupe (§8) → new picture (upload flow + target-dir `onAdd`/mirror auto-tag) or retag existing        |
| `PUT` (existing path)     | overwrite                                  | new `picture_version` **iff** `versioning_mode` allows, else in-place; copy staging→pictures; enqueue `gen_thumbnail`; ETag changes |
| `MOVE` (across dirs)      | re-file                                    | source-dir `onRemove` + target-dir `onAdd` (or mirror auto-tag); no body; identity = source path                                    |
| `MOVE` (same dir, rename) | rename                                     | rename `pictures.filename` (naming=`original`); it is a real sync endpoint, not just a view                                         |
| `COPY`                    | appear in two places                       | target-dir `onAdd` / mirror auto-tag (picture becomes multi-tagged); no byte copy                                                   |
| `DELETE`                  | per `safeDeleteMode`                       | `singleBranch` → `onRemove` (may `409`, §7.2); `fullDelete` → `deleted_at` (received pictures always local-only)                    |
| `MKCOL`                   | new directory                              | static/query → `405`; **drop → `405`** (leaf, feature 18 §4); mirror subtree → sidecar (§9)                                         |
| `PROPPATCH`               | client sets mtime, etc.                    | accept as no-op (clients need a success)                                                                                            |
| `LOCK`/`UNLOCK`           | Finder class 2                             | in-memory lock store                                                                                                                |

### 7.2 Write-back & conflicts

Write-back uses the node's `writeBack` op-list (`query`) or the implicit assign/remove of
the directory's own tag (`mirror`), operating on **`manual`** tags only. Two hard failures:

- **`409 Conflict`** when an `onRemove` would need to drop a tag that a **live
  `rule`/`segment`/`share_mapping` service still asserts** for that picture (the picture
  would still match after the write). Names the conflicting service. This is the
  "non-manual tag survives" rejection the owner requested.
- **`403 Forbidden`** on read-only targets per the writability matrix
  ([`18_hierarchy_improvements.md`](18_hierarchy_improvements.md) §5.2, superseding the
  hierarchy spec §7.5): a target is writable when its **effective** write-back is on
  (per-node `writeBackEnabled` tri-state under the master ceiling) and it carries an op-list.
  `fullDelete` is allowed everywhere (no tag mutation).

**Feature 18 deltas:** **`drop`** inbox nodes are writable **even when `config.writeBack:
false`** (the one exemption to the master read-only ceiling); a `PUT`/`COPY`/`MOVE`-in ingests
or dedupes as usual then applies the drop's fixed `onAdd`. **`matchUntagged`** query nodes may
now carry a **free-form** op-list (the §7.2-of-05 compliance check is skipped — "untagged" is
not an include/exclude predicate; a surviving pipeline tag may keep the picture out of the
directory and can still raise the `409`).

### 7.3 Versioning on overwrite

Overwrite-PUT consults the user's `versioning_mode` (cached in the §3.3 auth entry):
`none` → overwrite in place; `original_copy` → snapshot once; `full_versioning` → snapshot
before each overwrite. This reuses the existing version-snapshot machinery from the upload
path.

## 8. Identity resolution

How a written file maps to a picture:

- **Explicit ops (overwrite-PUT, MOVE, COPY, DELETE):** the source/target **path** reverse-
  resolves through the directory's `naming` strategy to a `picture_id`. MOVE/COPY/DELETE
  carry no body, so no hash is available or needed.
- **New-content PUT to a non-existent path:** compute SHA-256 **inline** while streaming
  (§7). Then:
    - **Hash matches an existing owned, non-deleted picture** → it's a relocate/copy a dumb
      client expressed as a fresh upload. **No new picture, no re-upload** — just apply the
      target directory's `onAdd` / mirror auto-tag to the existing picture.
    - **Hash matches a recently-trashed (`deleted_at` set) owned picture** → **un-delete** it
      (clear `deleted_at`) and apply the target tags. This covers a naive client implementing
      a rename under `fullDelete` as `DELETE old` + `PUT new`.
    - **No match** → genuine new picture: full upload flow + `gen_thumbnail` + target tags.
- **ETag = `pictures.file_hash`.** Inline hashing makes it available immediately, so smart
  sync clients can detect "unchanged" and skip, and stateless clients (rclone) diff by
  ETag/size/mtime. We don't control client diffing — exposing accurate ETag/size/mtime in
  PROPFIND is the lever that makes sync efficient. A naive client that can't detect a rename
  degrades to delete+upload, which the hash dedupe above makes storage-safe.

## 9. Mirror auto-tag & MKCOL

`mirror` directories are tag-derived, so writes into them (including into not-yet-existing
sub-paths) translate to tag assignment:

- **PUT/COPY/MOVE into a path whose nearest existing ancestor is a `mirror` node:** the new
  trailing path segments form a tag suffix; assign the **deepest** tag
  `tagRoot + <segments>` (e.g. dropping into `Photos/Travel/NewPlace/` under mirror `Photos`
  → assign `Photos.Travel.NewPlace`; inclusive matching surfaces it under the intermediate
  dirs automatically, so no intermediate tags are stored). This is mirror write-back
  generalised to depth.
- **Segment slugification:** a new segment that isn't a valid tag label `[A-Za-z0-9_]` is
  **slugified** (runs of disallowed chars → `_`, trimmed; empty → `untitled`) rather than
  rejected, since a sync client (Finder) often writes into a folder it created with a default
  name like `dossier sans titre` before it can be renamed. Only a reserved (`SharedToMe`) prefix
  still rejects (`409`). Case collisions are folded per §10c.
- **`MKCOL`:** static/query → `405 Method Not Allowed` (structure is fixed). Under a mirror
  (or where the nearest existing ancestor is a mirror) → **sidecar**: record a transient
  `webdav:pendingdir:{hierarchy}:{path}` in Redis (short TTL) so PROPFIND shows the empty
  dir until a file lands and mints the real tag. GC by TTL if nothing arrives.

## 10. Case-sensitivity

Tags are case-sensitive ltree; macOS/Windows filesystems are case-insensitive. Three
sub-problems:

- **(a) Authored sibling nodes.** Make §11 sibling-name uniqueness in the hierarchy
  validator **case-insensitive** — reject two `query`/`static` siblings `Fav` vs `fav` at
  save. Cheap, prevents the problem at the source.
- **(b) Mirror-expanded tag dirs** colliding case-only (user has both `Travel.France` and
  `Travel.france`): cannot be forbidden (the underlying tags are case-distinct). **Surface +
  tolerate** — keep the webapp warning; WebDAV exposes both directories and accepts that a
  case-insensitive client sees undefined behaviour on those two. Rare.
- **(c) Write-side duplicate minting** (a client folds `Photos/travel` onto existing
  `Photos.Travel`): on write, do **case-insensitive sibling matching** — if a sibling tag
  differing only by case exists, **reuse the existing-cased tag**; reject minting a new
  case-colliding sibling (`409`). Applies to the §9 mirror auto-tag path too.

## 11. Dotfiles

Finder/Explorer PUT `.DS_Store`, `._*` (AppleDouble), `.localized`, `Thumbs.db`,
`desktop.ini`. These are not images. A PUT/PROPFIND for an ignore-listed name is **accepted**
(returning errors makes Finder hang) and the content is **sidecar**ed in Redis
(`webdav:sidecar:{hierarchy}:{path}` → bytes + mtime, short TTL) so a follow-up PROPFIND sees
it, but it never becomes a picture. The ignore-list is a small constant in the WebDAV layer.

## 12. Other edge cases

- **Read-only targets / `409` conflicts:** see §7.2.
- **Shared (`SharedToMe`) pictures:** read-only bytes (you can't write another user's
  picture); reads redirect/proxy through the owner's cross-instance presign; `fullDelete` is
  a local `deleted_at` only.
- **Windows redirector:** Basic auth requires HTTPS (or a registry flag) and has a default
  **50 MB** file-size cap (registry-tunable). Documented limitation, not worked around.
- **PROPFIND performance:** a large directory needs batched metadata (size, mtime, hash) —
  served from `list_pictures` and the cached expansion (§13).
- **Trashed pictures** are excluded from all listings (`deleted_at IS NULL`), as elsewhere.

## 13. Caching

The webapp caches the tree client-side; WebDAV must cache server-side. Cache the **expanded
directory tree** per hierarchy in Redis, keyed by `(hierarchy_id, config updated_at)` so a
config edit invalidates it. PROPFIND directory listings and `stat` reuse this; per-directory
picture **counts/listings** stay live (cheap predicate over `list_pictures`). The §3.3 auth
entry, §9 pending-dir markers, and §11 sidecars are the other Redis keys.

## 14. Database changes

Single migration file (`001_initial_schema.up.sql`, per coding guidelines — edit in place).
On `hierarchies`:

- `webdav_token_enc BYTEA` — AES-256-GCM blob (nonce‖ciphertext‖tag); `NULL` ⇒ no token.
- `webdav_use_redirect BOOLEAN NOT NULL DEFAULT true` — read strategy (§6).
- Update the table `COMMENT`.

No other schema changes (picture/tag/version model is already write-ready). After editing:
`cd back && cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare`.

## 15. Config

- `WEBDAV_MAX_UPLOAD_BYTES` (default e.g. 5 GiB) — write body guard (§4).
- The encryption key is derived from the existing `JWT_SECRET` (§3.2) — **no new secret
  env var**.

## 16. Module layout

Following [03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md):

```
domain/hierarchy.rs       # + slugify(name); case-insensitive sibling validation (§10a)
infra/crypto.rs           # + HKDF webdav key, AES-256-GCM encrypt/decrypt token
infra/s3.rs               # + Storage::put_object_file (stream temp file → S3) / get_object (proxy read)
repository/hierarchy.rs    # + token get/set, find_by_owner_and_slug (token validation)
services/hierarchy.rs      # resolver already here; + name↔picture resolution, write-back apply
services/vfs.rs            # VirtualFs trait + impl over the resolver (read/write/move/copy/delete/mkdir)
services/webdav.rs         # token issue/regenerate/validate; Redis auth cache
api/webdav.rs              # dav-server DavFileSystem + LockSystem adapter; Basic-auth layer; routes
api/user/hierarchies.rs    # + GET/POST/PATCH webdav token endpoints (§17)
state.rs / main.rs         # register /webdav routes; raised body limit on write routes
```

The `VirtualFs` returns plain domain types; the `dav-server` adapter in `api/webdav.rs` is
the only WebDAV-aware code.

## 17. API (token management — User JWT)

Added to `api/user/hierarchies.rs`, under `/api/authenticated/hierarchies/{id}`:

| Method  | Path                      | Description                                                                                                             |
|---------|---------------------------|-------------------------------------------------------------------------------------------------------------------------|
| `GET`   | `/{id}/webdav`            | `{ url, token, use_redirect, enabled }` — mounts the slug; mints a token if none exists. `token` decrypted for display. |
| `POST`  | `/{id}/webdav/regenerate` | Rotate the token (invalidates the Redis cache and any mounted client). Returns the new `{ url, token }`.                |
| `PATCH` | `/{id}/webdav`            | `{ use_redirect }` — toggle read strategy.                                                                              |

`url` is `{scheme}://{back_domain}/webdav/{slug}` where `slug = slugify(name)`.

The WebDAV protocol surface itself lives at `/webdav/{slug}` (Basic auth, §3) — outside
`/api/authenticated`, since clients can't send a User JWT.

## 18. Testing

- **domain:** `slugify`; case-insensitive sibling-name rejection (§10a).
- **infra/crypto:** token encrypt→decrypt round-trip; HKDF key stability; tamper → error.
- **repository:** token get/set; `find_by_owner_and_slug`.
- **services/vfs:** `list_dir`/`stat` projection (naming, collisions); write-back apply
  (`onAdd`/`onRemove`, mirror auto-tag deepest-only); identity resolution (path vs hash
  dedupe, un-delete on rematch); `409` on non-manual tag survival; `403` read-only.
- **api/webdav:** Basic auth (valid/invalid/disabled); PROPFIND Depth 0/1, infinity→403;
  GET redirect vs proxy; PUT new/overwrite; MOVE/COPY/DELETE; MKCOL 405 vs sidecar; dotfile
  sidecar.

## 19. Out of scope (future)

- **SFTP adapter** over the same `VirtualFs` (the trait exists for this).
- **Tag-rename cascade into hierarchy `config`** — still the stubbed `TaskQueue::TagRename`
  task; tag rename remains unsupported (shared with the hierarchies spec).
- **Per-device WebDAV tokens** — a `webdav_tokens` table for labelled/independently-revocable
  tokens. The single-token model is the MVP.
- **Auto-created default hierarchy** — not created on signup (would double-sync).

## 20. Documentation to update

- **[06_API_REFERENCE.md](../06_API_REFERENCE.md):** the WebDAV token endpoints (§17) and a
  short WebDAV protocol section (mount URL, Basic auth, behaviour notes).
- **[03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md):** the new modules (§16);
  the `/webdav/*` route group.
- **[01_GENERAL_SPECIFICATIONS.md](../01_GENERAL_SPECIFICATIONS.md) §4:** note the write
  endpoints now ship (WebDAV) and the auth/identity model.
- **[99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md):** tick the WebDAV item.
- **[05_hierarchies.md](05_hierarchies.md) §13:** the WebDAV out-of-scope item is now this
  spec.

## 21. Implementation status & MVP deviations

The backend is implemented and tested (`services/vfs.rs`, `services/webdav.rs`,
`api/webdav.rs`, the token API in `api/user/hierarchies.rs`, schema + crypto + resolver
support). It deviates from the design above in a few deliberate MVP simplifications:

- **Hand-rolled handler, not `dav-server` (§4).** The `dav-server` crate has no redirect hook
  for GET and would fight the http/body types; since reads redirect and writes stream with
  inline hashing, a focused hand-rolled handler over `VirtualFs` is more controllable. The
  property set is fixed (displayname/resourcetype/getcontentlength/getlastmodified/
  getcontenttype/getetag), so PROPFIND/PROPPATCH XML is built directly.
- **Key derivation (§3.2)** is SHA-256 domain-separated, not HKDF (avoids a second digest
  dependency); functionally equivalent for a high-entropy secret.
- **Locking** is advisory/fake (LOCK returns a token, nothing is enforced) — enough for
  Finder's class-2 requirement.
- **PUT streams to a temp file** (06_webdav.md §7): the request body is streamed to a temporary
  file — never buffered in memory — while its size is enforced against `WEBDAV_MAX_UPLOAD_BYTES`
  (a real env config, default 5 GiB); the finished file is hashed with the common crate's chunked
  `hash_file` (the hash is needed before deciding whether to upload or just retag), then streamed
  to S3 via `Storage::put_object_file` (`ByteStream::from_path`, no in-memory copy). The inline
  hash is persisted immediately (`PictureRepository::set_file_hash`) so the ETag is correct and a
  quick re-upload dedupes before the worker runs. A zero-byte PUT is accepted but ingests nothing
  (Finder/Explorer issue an empty placeholder PUT before the real bytes), so empty objects never
  reach S3 or the picture table.
- **Versioning-on-overwrite (§7.3) is implemented** — an overwrite-PUT consults the user's
  `versioning_mode` and snapshots the current bytes as a `picture_version` before replacing them
  (`none` → never; `original_copy` → first overwrite only; `full_versioning` → every overwrite),
  reusing the worker edit path's snapshot machinery (`pictures::snapshot_version_on_overwrite`).
- **OS-junk sidecars are implemented (§11)** — AppleDouble (`._*`, incl. `._.`), `.DS_Store`,
  `.localized`, `Thumbs.db`, `desktop.ini`, and similar are never ingested as pictures. Instead
  they are stored as transient **Redis sidecars** (`webdav:sidecar:{hierarchy}:{parent}` → a
  name→bytes map, day TTL): a `PUT` stores the bytes, `GET` serves them back, `PROPFIND`/listings
  echo them (via `Vfs::stat`/`list_dir`), and `DELETE` removes them. Oversized bodies (>1 MiB) are
  accepted but not stored. This keeps sync clients happy without polluting the tag/picture model.
- **Brand-new mirror subdir auto-tag is implemented (§9)** — a `PUT`/`COPY`/`MOVE` into a path
  whose nearest existing ancestor is a writable `mirror` node mints the **deepest** tag
  (`tagRoot + new segments`). New segments are **slugified** to valid tag labels via
  `TagPath::slugify_label` (Finder's `dossier sans titre` → `dossier_sans_titre`) rather than
  rejected — a sync client can't always rename a folder before its first write. Only a reserved
  (`SharedToMe`) prefix still `409`s. `MKCOL` under a mirror records a transient **Redis
  pending-dir** marker (`webdav:pendingdir:{hierarchy}:{parent}`, day TTL) under the folder's
  *original* name so PROPFIND shows the empty directory until a file lands and mints the slugified
  tag (the marker is then cleared / GC'd by TTL). The empty-folder lifecycle is fully wired: a
  pending dir can be renamed (`MOVE`) or removed (`DELETE`) by moving/dropping its marker, so
  Finder's create→rename→drop flow works. `MKCOL` outside a mirror or on an existing path is
  rejected (`403`/`409`). `ResolvedDir` carries the new `mirror_tag` to drive this.
- **Case-insensitive write-side tag reuse (§10c) is implemented** — on write, each assigned tag
  path is folded onto an existing case-variant tag (`domain::hierarchy::reuse_existing_case`), so a
  case-insensitive client never mints a case-only-duplicate sibling. Authored sibling
  case-insensitivity (§10a) and the mirror collision tolerance (§10b) are also in place.
- **Frontend** token UI is not yet built (part of the broader "Full frontend" roadmap item).
- **Integration tests** cover the pure helpers (auth parsing, naming/collision, slug, crypto,
  PROPFIND href/XML) and the end-to-end VFS read/write taxonomy against a seeded DB
  (`back/tests/vfs.rs`): list/stat/proxy-and-redirect reads; PUT new/overwrite/dedupe/un-delete/
  empty; MOVE/COPY/DELETE (singleBranch + fullDelete); the `409` non-manual-tag-survival path;
  versioning-on-overwrite; the §10c case-fold; the §9 brand-new mirror subdir auto-tag
  (PUT/COPY/multi-level/MKCOL-then-PUT, label slugification incl. the Finder untitled-folder
  create→rename→PUT flow, outside-mirror rejection); and the §11 sidecar round-trip.
