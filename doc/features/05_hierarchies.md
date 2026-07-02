# Hierarchies

## 1. Overview & goals

Roadmap item **"Hierarchies"** ([99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md)): CRUD for
user-defined hierarchies, plus the **resolver** that turns a hierarchy into a navigable
directory tree. A hierarchy is a saved, customisable mapping from the user's tag graph to a
filesystem-like tree, used by **two front-ends**: the webapp sidebar/navigation and (later)
WebDAV.

Core invariant: **a hierarchy stores no pictures.** Every directory is a pure function of
the tag graph and resolves to a **tag-set predicate**. Picture membership, counts, and
listings are always derived live. This makes a hierarchy a *view*, never a copy.

Scope of this spec:

- The hierarchy **data model** (`config` JSONB) and its validation.
- The **read resolver**: building the directory tree and the per-directory picture
  predicate ("most-specific node wins").
- The generalised **`list_pictures`** tag query (`include`/`exclude`/`match`/`untagged`)
  and the internal `TagPredicate`.
- CRUD + navigation **API** (`tree`, `browse`).
- The **write-back model** (op-lists, compliance, conflicts) — specified in full so the
  schema is write-ready, but the **write endpoints themselves ship with WebDAV** and are
  out of scope here (§13).

Out of scope (tracked elsewhere): the WebDAV server, tag-rename cascade into hierarchy
config (the cascade task is stubbed — [`infra/tasks.rs`](../../back/src/infra/tasks.rs)),
file name↔picture resolution mechanics under WebDAV, and any auto-created default
hierarchy.

## 2. Decisions (settled)

- **Hybrid node tree (§3).** `config` is an ordered tree of nodes. Three kinds: `mirror`
  (dynamic tag-tree expansion), `query` (explicit tag predicate, may have children),
  `static` (pure container). No relational `hierarchy_nodes` table — JSONB, no GIN index
  (users have ~2–5 hierarchies).
- **Most-specific node wins (§5.3).** A directory lists pictures that match it but match
  **none of its visible children**: `direct(D) = P(D) ∧ ⋀ᵢ ¬own(childᵢ)`. No
  parent/child duplication; pictures may live at any level. This single rule covers both
  mirror and query directories.
- **`exclude` everywhere, `collapsed` mirror-only (§4).** `exclude` is the one membership
  cut (the former `disabled` is `exclude` on a mirror node, which prunes both pictures and
  directories). `collapsed` is the mirror-only roll-up.
- **Untagged via `query` (§5.5).** Empty `include` ⇒ "all pictures"; `matchUntagged: true`
  ⇒ "no stored tag of any source". `SharedToMe` (incoming-share) tags count as tagged. No
  dedicated `untagged` kind, no path-grammar sentinel.
- **`match: all | any` per query node (§4).** One combinator over a flat include list, plus
  `exclude` = "reject if the picture has any excluded tag". Richer boolean logic comes from
  the **node tree** (AND down via inheritance, OR across sibling directories), not a
  per-node expression language. No additional filter fields.
- **Backend-only navigation (§9).** `tree` (directories) and `browse` (paginated pictures)
  endpoints. The complex per-directory predicate never leaves the backend. The public
  `GET /pictures` gains a *flat* `include_tags/exclude_tags/match/untagged` (no
  minus-children) for general use (saved searches).
- **Write-back op-lists (§7).** Each writable node declares `writeBack: { onAdd, onRemove }`
  tag operations. The edit service validates they can satisfy/break the read predicate. A
  `query` node is writable **iff `writeBack` is non-null**; `null` ⇒ read-only. `mirror`
  write-back is implicit (its own tag).
- **`safeDeleteMode` (§7.4).** `singleBranch` applies `onRemove`; `fullDelete` trashes the
  picture (`deleted_at`). `fullDelete` performs no tag mutation, so it is allowed on **any**
  directory (read-only included) and can never raise a 409.
- **No `sort` in config.** Ordering is a client/request concern; `browse` takes the same
  `sort`/`order`/`page` params as the existing list endpoint.

## 3. Conceptual model

A hierarchy is an ordered tree of **nodes**. Each node renders to a **directory**. A node
carries (directly or by derivation) a **read predicate** `P` over a picture's stored tag
set. Two families:

- **Dynamic** (`mirror`): expands into a *tag-derived* subtree at resolve time. It is a
  **leaf in the authored JSON** — its descendant directories come from the live tags under
  `tagRoot`. You cannot hand-author children under a mirror node, but you can place a mirror
  node anywhere among explicit nodes.
- **Explicit** (`query`, `static`): author-defined `children`. A `query` node's effective
  read predicate is **`own ∧ all ancestors`** (inheritance, as segmentation `requires`
  does). A `static` node is a pure container with no predicate of its own.

**Membership inclusivity.** All tag matching is inclusive of descendants, matching the rest
of the system: a picture with stored tag `Photos.Travel.Alps` satisfies a predicate term
`Photos` (`tag_path <@ 'Photos'`). This mirrors `picture_has_tag` in the schema and the
tagging-service gate.

## 4. `config` JSONB schema

Stored in `hierarchies.config`. Table columns `name`, `enabled`, `created_at`,
`updated_at` are unchanged. `owner_id` + `uq_hierarchy_name (owner_id, name)` unchanged.

### 4.1 Top level

```jsonc
{
  "version": 1,                       // schema version of this blob, for forward migration
  "safeDeleteMode": "singleBranch",   // "singleBranch" | "fullDelete"  (hierarchy default)
  "naming": "original",               // "original" | "date" | "id"     (hierarchy default)
  "writeBack": true,                  // master switch; false ⇒ entire hierarchy read-only
  "nodes": [ /* ordered Node[] */ ]   // explicit root-level tree
}
```

### 4.2 Node — common fields

```jsonc
{
  "id": "n_ab12",        // stable, unique-within-hierarchy id. Sidebar keys, write-path
                         //   resolution when sibling names need disambiguation, reorder.
  "kind": "mirror",      // "mirror" | "query" | "static"
  "name": "Photos",      // directory label. `mirror`: optional display override of the
                         //   tagRoot's last label (defaults to it). Others: required.
  "naming": null,        // optional per-node override of the hierarchy `naming`
  "safeDeleteMode": null // optional per-node override of the hierarchy default
}
```

### 4.3 `kind: "mirror"`

Mirrors the tag subtree under `tagRoot`. Keeps the collapse/exclude controls.

```jsonc
{
  "id": "n_photos", "kind": "mirror", "name": "Photos",
  "tagRoot": "Photos",        // ltree prefix this node mirrors
  "keepDir": false,           // false ⇒ tagRoot's own label is stripped (its contents sit at
                              //   the node root); true ⇒ tagRoot appears as a directory level
  "collapsed": ["Photos.Travel.Alps.Hiking"],  // subtrees collapsed; their pictures bubble up
                                               //   to the nearest enabled ancestor directory
  "exclude":   ["Photos.Outdoor"]              // subtrees removed entirely (pictures + dirs)
  // no `children`: subtree is tag-derived.
  // write-back is implicit (assign/remove the directory's own tag); not stored.
}
```

Validation: every `collapsed`/`exclude` entry must be `<@ tagRoot`.

> **Extended in [`18_hierarchy_improvements.md`](18_hierarchy_improvements.md) §7:** a mirror
> gains `maxDepth` (cap directory generation N levels below `tagRoot`) + `deeperMode`
> (`collapse|exclude` for pictures below the cut), and `exclude` entries may now be **foreign**
> to `tagRoot` (a pure picture-membership cut, no directory effect); `collapsed` stays
> `<@ tagRoot`.

### 4.4 `kind: "query"`

Explicit predicate; may nest.

```jsonc
{
  "id": "n_fav", "kind": "query", "name": "Favorites",
  "match": "all",                     // "all" (AND) | "any" (OR). Empty include ⇒ matches ALL.
  "include": ["Photos", "Starred"],   // plain ltree path strings
  "exclude": ["Photos.Private"],      // reject if the picture has any of these (inclusive)
  "matchUntagged": false,             // true ⇒ "no stored tag of any source"; mutually
                                      //   exclusive with include/exclude (must be empty)
  "writeBack": {                      // null/omitted ⇒ read-only. UI auto-derives defaults.
    "onAdd":    [ { "op": "assign", "path": "Starred" } ],
    "onRemove": [ { "op": "remove", "path": "Starred" } ]
  },
  "children": [ /* Node[]; effective read predicate = own ∧ ancestors */ ]
}
```

### 4.5 `kind: "static"`

Pure container — no predicate, no direct pictures, read-only.

```jsonc
{ "id": "n_albums", "kind": "static", "name": "Albums",
  "children": [ /* Node[] */ ] }
```

### 4.6bis `kind: "drop"` (feature 18)

A write-only inbox: always shown, lists nothing, and applies a fixed `onAdd` op-list to every
upload. Always writable (ignores `writeBackEnabled` and the master switch). Full spec:
[`18_hierarchy_improvements.md`](18_hierarchy_improvements.md) §4.

```jsonc
{ "id": "n_inbox", "kind": "drop", "name": "Inbox",
  "onAdd": [ { "op": "assign", "path": "Inbox" } ] }
```

### 4.7 Per-node `writeBackEnabled` (feature 18)

Every node gains an optional `writeBackEnabled: bool | null` (`null` = inherit the nearest
explicit ancestor, root seed = the master switch). The master switch stays a hard ceiling. See
[`18_hierarchy_improvements.md`](18_hierarchy_improvements.md) §5. `version` is bumped to `2`;
v1 blobs deserialize forward unchanged.

### 4.6 Changes vs. the current schema default

The migration's `hierarchies.config` default and `COMMENT` are replaced with the §4.1 shape
(`version`, `nodes`, `safeDeleteMode`, `naming`, `writeBack`). The former
`roots/collapsedTags/disabledTags` flat keys are gone (folded into per-node `mirror`
fields). **Drop `idx_hierarchies_config`** (the GIN index) — no query needs it; hierarchies
are always loaded whole by `id`/`owner_id`. `idx_hierarchies_owner` stays.

## 5. Read resolution

The resolver lives in `services::hierarchy` and is the single source of truth for both
front-ends. It has two responsibilities: build the **directory tree** (cheap, no pictures)
and compute a **per-directory `TagPredicate`** (fed to `list_pictures`).

### 5.1 Inputs

- The hierarchy `config`.
- The user's **distinct tag paths** — `TagRepository::list_paths_by_user` already returns
  `DISTINCT tag_path` for non-deleted pictures. This is the skeleton for mirror expansion
  and for "which directories exist".

### 5.2 Building the directory tree

Walk `nodes` depth-first, producing directory nodes addressed by a **path of segment
names** (§5.6):

- `static` / `query` → a directory named `name`, with authored `children` recursed.
- `mirror` → expand against the distinct tag paths under `tagRoot`:
    1. Take all distinct paths `p` with `p <@ tagRoot`, minus any under an `exclude` subtree.
    2. Apply `collapsed`: a path inside a collapsed subtree contributes its pictures to the
       **nearest enabled ancestor** directory but generates **no directory** of its own.
    3. The remaining paths form a tree of directories. `keepDir=false` strips the `tagRoot`
       label so the root's children sit at the mirror node's level; `keepDir=true` keeps
       `tagRoot` as one directory level (named `name`).
    4. **Container directories** with no exact tag still exist if a deeper path requires them
       (e.g. `Photos.Travel.Alps` with nothing tagged exactly `Photos.Travel` still yields a
       `Travel` directory). Derived from the path set, independent of exact-tag membership.
- **Empty directories are hidden** (§12): a directory whose entire subtree resolves to zero
  visible pictures is omitted. (For the `tree` endpoint this is computed from path presence;
  precise zero-picture pruning may be deferred to the `counts=true` path to stay cheap.)

### 5.3 Per-directory picture predicate — "most-specific node wins"

A picture is a **direct file** of directory `D` iff it matches `D` and matches none of `D`'s
**visible** children:

```
direct(D) = P(D) ∧ ⋀ᵢ ¬own(childᵢ)
```

**Mirror directory `D` (tag path `T`)** — derived form of the above:

- `P(D)` membership = a picture has a stored tag `= T` **or** under a subtree collapsed into
  `D`. (Note: collapsed roll-ups are an **OR of prefixes**, not a flat AND — this is why the
  predicate is built backend-side and not expressible as the public flat filter.)
- `own(childᵢ)` = the immediate visible child directories of `D` (each a tag path `Cᵢ`
  that is enabled and not collapsed-into-`D`). Excluded subtrees and collapsed subtrees do
  **not** appear here.
- Therefore the SQL shape is:

  ```sql
  EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path = $T)          -- exact T
  -- (∪ OR arms for each collapsed-into-D subtree root: t.tag_path <@ $collapsedRoot)
  AND NOT EXISTS (                                                                      -- no deeper visible child
    SELECT 1 FROM tags t2
    WHERE t2.picture_id = p.id
      AND t2.tag_path <@ $T AND t2.tag_path <> $T
      AND <t2 lands in a visible child dir>   -- i.e. t2 <@ some visible child Cᵢ
  )
  ```

  **Why `tag_path = T` is not enough, and we must check "no deeper visible child":** tags
  are stored per-source (two partial unique indexes — `uq_picture_tag_manual` and
  `uq_picture_tag_source`). A picture can simultaneously carry a `manual` tag `Photos.Travel`
  **and** a `rule` tag `Photos.Travel.France`. It has `tag_path = Photos.Travel` in the DB,
  yet `Photos.Travel` is **not** its deepest path. Folding to deepest distinct paths
  (`TagPath::fold_deepest`) drops `Photos.Travel`, so the picture belongs in `France`, not
  directly in `Travel`. The `NOT EXISTS deeper-visible-child` clause enforces exactly this.

**Query directory `D`:**

- `P(D)` = inherited predicate: `own(D) ∧ all ancestors`, where `own(D)` = `match`-combined
  `include` (inclusive `<@`), AND none of `exclude` (inclusive `<@`), OR the `matchUntagged`
  predicate when set.
- minus each visible child's `own` predicate.

### 5.4 Collapsed vs. exclude (precise)

- **`exclude` (T):** a picture with **only** tags under `T` disappears from the hierarchy;
  the directories for `T`'s subtree are not generated. If the picture also has a tag outside
  `T`, it still shows under that other directory.
- **`collapsed` (T):** the directory for `T` (and its descendants) is not generated; pictures
  tagged under `T` surface in the nearest **enabled** ancestor directory of `T`.

### 5.5 Untagged

- Empty `include` ⇒ `own` membership is vacuously true (all pictures), so the **complement
  pattern** works: `include: []`, `exclude: ["Photos"]` ⇒ "everything not filed under
  Photos".
- `matchUntagged: true` ⇒ `own = NOT EXISTS (any tag row for this picture)`. Robust and
  cheap; avoids enumerating roots. A received picture carrying a `SharedToMe…`
  `incoming_share` tag **counts as tagged** and is therefore excluded.

### 5.6 Path addressing

A directory is addressed by a `path` of **segment names**: explicit-node `name`s down the
authored tree, then tag **labels** inside a `mirror` expansion (the `tagRoot` label present
only when `keepDir=true`). Uniqueness is guaranteed by validation (§11 — unique sibling
names, including mirror-expanded names vs. explicit siblings). Endpoints return child
segment names; the client appends. **Names, not ids**, are used on the wire (WebDAV
compatibility); node `id`s are internal only.

## 6. `TagPredicate` and `list_pictures` changes

### 6.1 Internal `TagPredicate` (domain)

A new pure type in `domain` (e.g. `domain::hierarchy` or alongside `domain::tag`) the
resolver builds and the repository renders to SQL:

```rust
pub struct TagPredicate {
    pub include: Vec<TagPath>,     // empty ⇒ "all"
    pub match_all: bool,           // true = AND, false = OR (ignored when include empty)
    pub exclude: Vec<TagPath>,     // reject if picture has any (inclusive)
    pub untagged: bool,            // strict no-tag
    pub exact: Vec<TagPath>,       // mirror exact-T arms (tag_path = T, non-inclusive)
    pub minus_children: Vec<TagPredicate>,  // own(childᵢ) terms to subtract (most-specific-wins)
}
```

(`exact` + the `<@` collapsed arms together encode mirror membership; query nodes use
`include`/`match_all`/`exclude`/`untagged`. The same struct serves both.)

### 6.2 Repository

Extend `PictureRepository::push_filters`
([`repository/picture.rs`](../../back/src/repository/picture.rs)). Today it supports a single
`filter.tag` (`tag_path <@ $`). Add the predicate rendering:

- each `include` (or the `match_all`/`any` combination) → `EXISTS … <@`;
- each `exact` → `EXISTS … tag_path = $`;
- each `exclude` → `NOT EXISTS … <@`;
- `untagged` → `NOT EXISTS (SELECT 1 FROM tags WHERE picture_id = p.id)`;
- each `minus_children[i]` → `AND NOT ( <rendered child predicate> )`.

`PictureListFilter` gains the `TagPredicate` (the existing `tag: Option<String>` becomes a
convenience that lowers to `include: [tag]`). The count query and the page query share
`push_filters`, so both stay consistent.

### 6.3 Public `GET /pictures` (flat subset)

`PictureListParams` ([`services/pictures.rs`](../../back/src/services/pictures.rs)) gains:

- `include_tags: Vec<String>` (repeatable) — AND/OR per `match`.
- `exclude_tags: Vec<String>` (repeatable).
- `match: "all" | "any"` (default `all`).
- `untagged: bool`.

These map to a `TagPredicate` with **no** `exact`/`minus_children` (flat). `tag` stays as a
single-include alias. Hierarchy depth (exact arms, minus-children) is **only** produced by
the resolver for `browse`, never accepted from the client.

## 7. Write-back (model only — endpoints ship with WebDAV, §13)

A directory's read predicate `P` defines membership; writing must make a picture satisfy
(`onAdd`) or stop satisfying (`onRemove`) `P`. Because `P` can be an arbitrary
`match`/`include`/`exclude` combination, the operations are **declared explicitly** per node
rather than inferred at write time.

### 7.1 Op-list

```jsonc
"writeBack": {
  "onAdd":    [ { "op": "assign" | "remove", "path": "<ltree>" }, ... ],
  "onRemove": [ { "op": "assign" | "remove", "path": "<ltree>" }, ... ]
}
```

- `mirror`: implicit — `onAdd = [assign T]`, `onRemove = [remove T]` (T = the directory's
  tag). Not stored.
- `query`: stored. The **UI auto-derives** defaults from the predicate; the author may
  override.

### 7.2 Compliance (validated by the edit service — see §11)

Let `INC` = include paths, `EXC` = exclude paths. A picture **matches** when: (`match:all` →
has every `INC`) or (`match:any` → has ≥1 `INC`); **and** has none of `EXC`. Validation
checks the op-list is *structurally capable*:

- **`onAdd`** must, together: assign **all** `INC` (`match:all`) or **≥1** `INC`
  (`match:any`); **and** `remove` every `EXC` (so the added picture is not excluded). Empty
  `INC` ⇒ no include op needed.
- **`onRemove`** must do **at least one** breaking op: `remove` ≥1 `INC` (`match:all`) or
  `remove` **all** `INC` (`match:any`); **or** `assign` one `EXC`.

### 7.3 Runtime conflict (409)

Write-back operates on **`manual`** tags only (`TagRepository::batch_assign` /
`batch_remove` already restrict to `source = 'manual'`). If an `onRemove` drops a manual tag
that a **live `rule`/`segment` service still asserts** for that picture (or an `EXC` cannot
be cleared because a service re-adds it), the picture still matches after the write → return
**`409 Conflict`** naming the conflicting service (general spec §4.3). This is the only
runtime write failure for `singleBranch`.

### 7.4 `safeDeleteMode`

- **`singleBranch`**: apply `onRemove`. May 409 (§7.3). The recommended default for
  third-party sync clients (general spec §7 — dumb clients deleting a multi-tag picture from
  every visible path).
- **`fullDelete`**: mark the picture `deleted_at` (trash). **No tag mutation**, so **no 409
  is possible** and it is allowed on **any** directory, including read-only ones. Received
  (shared) pictures are always `deleted_at`-local, never physical (general spec §2).

### 7.5 Writability matrix

> **Superseded by [`18_hierarchy_improvements.md`](18_hierarchy_improvements.md) §5.2.**
> Write-back became a per-node tri-state (`writeBackEnabled: inherit|on|off`) under the
> hierarchy master switch (still a hard ceiling), the `drop` inbox kind was added (always
> writable, even master-off), and `matchUntagged` queries may now carry a (free-form) op-list.
> The table below is the original v1 model.

| Node kind                     | add / copy / move-in / singleBranch delete | fullDelete            |
|-------------------------------|--------------------------------------------|-----------------------|
| `mirror`                      | writable (implicit op-list)                | allowed               |
| `query`, `writeBack` non-null | writable                                   | allowed               |
| `query`, `writeBack` null     | **read-only** (`403`)                      | allowed               |
| `query` `matchUntagged`       | read-only (`403`)                          | allowed               |
| `static`                      | read-only (`403`)                          | n/a (no direct files) |

`config.writeBack: false` forces the entire hierarchy read-only (fullDelete still allowed)
— **except `drop` nodes** (feature 18 §5.4).

## 8. Naming strategy

Per-node (`naming`) with a hierarchy default. Used to present a stable, collision-free file
name under WebDAV; the **webapp ignores it** (shows real `filename` + thumbnails).

- `original` — `pictures.filename`; on collision within a directory, append the first 6
  chars of the picture id (stable, unlike an ordinal `-2`).
- `date` — from `captured_at` (e.g. `2024-08-01_153012.jpg`); same id disambiguation.
- `id` — `{picture_id}.{ext}`; always unique.
- Fallback to `id` when `filename`/`captured_at` is null.

**Identity** (resolving a WebDAV write back to a picture) uses `file_hash` to recognise an
existing picture being moved/renamed (avoid double-upload), falling back to the display name
only for the content-changed case. `file_hash` remains the WebDAV **ETag**. The exact
name↔id resolution lands with WebDAV (§13).

## 9. API

All under `/api/authenticated/hierarchies`, User JWT. Added to
[`api/user.rs`](../../back/src/api/user.rs) (`authenticated_routes`); handlers in a new
`api/user/hierarchies.rs`.

### 9.1 CRUD

| Method   | Path                | Description                                                 |
|----------|---------------------|-------------------------------------------------------------|
| `GET`    | `/hierarchies`      | List the user's hierarchies (id, name, enabled).            |
| `POST`   | `/hierarchies`      | Create. Body: `{ name, config }`. Validates `config` (§11). |
| `GET`    | `/hierarchies/{id}` | Get one with full `config`.                                 |
| `PATCH`  | `/hierarchies/{id}` | Update `{ name?, enabled?, config? }`. Re-validates.        |
| `DELETE` | `/hierarchies/{id}` | Delete.                                                     |

### 9.2 Navigation (read resolver)

| Method | Path                       | Description                                                                                                                                                                            |
|--------|----------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `GET`  | `/hierarchies/{id}/tree`   | Directories only. Query: `path` (default root), `depth` (default 1), `counts`.                                                                                                         |
| `GET`  | `/hierarchies/{id}/browse` | Paginated **pictures** of one directory. Query: `path`, `page`, `page_size`, `sort`, `order`, `include_deleted`, `owned_only`, `shared_with_me`, `captured_after/before`, `thumbnail`. |

**`tree` response** (cheap; sidebar):

```jsonc
{
  "path": "Photos/Travel",
  "directories": [
    { "name": "Alps", "writable": true, "child_count": 2, "picture_count": null }
  ]
}
```

`child_count` is always returned (derived from the distinct-tag set). `picture_count` is
`null` unless `?counts=true` (it requires the direct-files predicate per directory — the
expensive part).

**`browse`** resolves `path` → `TagPredicate` via the resolver, then reuses
`list_pictures` with the existing pagination/filters. Response is the existing
`PictureListResult`. The hierarchy predicate is built **server-side**; the client only ever
sends a `path`.

### 9.3 Write operations

Deferred to WebDAV (§13). The `config` already encodes everything needed (§7); no write
endpoints are added in this item.

## 10. Database changes

Single migration file (`001_initial_schema.up.sql`, per coding guidelines — edit in place):

- Replace the `hierarchies.config` `DEFAULT` with the §4.1 shape.
- **Drop `idx_hierarchies_config`** (GIN). Keep `idx_hierarchies_owner`,
  `uq_hierarchy_name`, the `update_hierarchies_updated_at` trigger.
- Update the table `COMMENT` to describe the node-tree config.

No new tables, types, or columns. After editing:
`cd back && cargo sqlx migrate revert && cargo sqlx migrate run && cargo sqlx prepare`.

## 11. Validation (domain, pure)

Config validation lives in `domain` (e.g. `domain::hierarchy`) and is called by the service
on create/patch. Rules:

- Every tag path parses via `TagPath` (ltree labels `[A-Za-z0-9_]`). Paths may be under
  `SharedToMe` (E5 — received pictures are first-class; write-back there stays
  prefix-protected by the tag layer).
- `mirror.collapsed[i] <@ tagRoot` and `mirror.exclude[i] <@ tagRoot`.
- Node `id`s unique within the hierarchy.
- **Sibling `name`s unique**, including a `mirror` node's expanded top-level names vs.
  explicit siblings → reject at save (no auto-merge/auto-suffix).
- `matchUntagged: true` ⇒ `include` and `exclude` empty.
- `writeBack` (when present) passes the §7.2 compliance check against the node's predicate.
- `static` nodes have no predicate fields; `mirror` nodes have no `children`.
- `safeDeleteMode` ∈ {`singleBranch`,`fullDelete`}; `naming` ∈ {`original`,`date`,`id`}.

## 12. Module layout

Following [03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md):

```
domain/hierarchy.rs       # HierarchyConfig + Node types, validation, TagPredicate
repository/hierarchy.rs    # CRUD SQL (load/store config), reuse list_paths_by_user
repository/picture.rs      # extend push_filters / PictureListFilter with TagPredicate
services/hierarchy.rs      # resolver: build_tree, predicate_for_path; CRUD orchestration
api/user/hierarchies.rs    # CRUD + tree + browse handlers + request/response models
api/user.rs                # register routes
```

The resolver returns plain domain types so a future WebDAV adapter and the webapp browse
endpoint both consume it.

## 13. Out of scope (future)

- **WebDAV server** (own roadmap item): the bidirectional filesystem over this resolver —
  GET/PUT/MOVE/COPY/DELETE, presigned reads, staging-pattern writes, ETag = `file_hash`,
  the name↔picture resolution (§8), and the write endpoints exercising §7. The data model
  here is built write-ready so WebDAV adds no schema.
- **Tag-rename cascade into `config`**: rewriting `tagRoot`/`include`/`exclude`/`collapsed`
  on rename is part of the stubbed `TaskQueue::TagRename` task
  ([`infra/tasks.rs`](../../back/src/infra/tasks.rs)); tag rename is unsupported for now.
- **Case-insensitive client divergence** (general spec §7): a client folding
  `Photos/travel` onto `Photos.Travel` can mint a case-duplicate tag on push. Mitigation is
  a WebDAV-write-layer concern (case-insensitive sibling match or `409`); for now the webapp
  may warn when a hierarchy has case-only sibling tags.
- **Offline owner**: shared pictures from an offline instance are unreadable — the webapp
  shows the blurhash. Expected, documented limitation.
- **Auto-created default hierarchy**: not created on signup (would double-sync under WebDAV);
  the user builds hierarchies explicitly.

## 14. Edge cases

- **Multi-source same-path tags** drive the "no deeper visible child" rule (§5.3) — the
  governing correctness case; cover with tests.
- **Container directories** without an exact tag still navigable (§5.2).
- **Empty directories** hidden (§5.2).
- **Deleted/trashed pictures** always excluded (`deleted_at IS NULL`), as in
  `list_paths_by_user` and `get_pictures_under_tag`.
- **A picture in several sibling directories** (multiple deepest tags) is expected; it is the
  duplicate-delete hazard `singleBranch` mitigates (§7.4).
- **Reserved characters**: tag labels are `[A-Za-z0-9_]`, so directory names are
  filesystem-safe by construction; only file names (§8) need care.

## 15. Testing

- **domain**: `TagPath` predicate parsing; config validation (sibling-name collision,
  `collapsed/exclude <@ tagRoot`, `matchUntagged` exclusivity, write-back compliance for
  `match:all`/`match:any`/exclude).
- **repository**: `push_filters` rendering of `include`/`exclude`/`exact`/`untagged`/
  `minus_children`; the multi-source "deepest wins" case; collapsed roll-up; exclude pruning.
- **services**: resolver `build_tree` (keepDir, collapsed, exclude, container dirs, empty-dir
  hiding) and `predicate_for_path` for mirror and nested query directories.
- **api**: CRUD validation errors; `tree` (counts on/off); `browse` pagination + filters.

## 16. Documentation to update

- **[06_API_REFERENCE.md](../06_API_REFERENCE.md)**: hierarchies CRUD + `tree`/`browse`; the
  new `include_tags/exclude_tags/match/untagged` params on `GET /pictures`.
- **[03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md)**: the new modules (§12) in
  the layout.
- **[01_GENERAL_SPECIFICATIONS.md](../01_GENERAL_SPECIFICATIONS.md) §4**: align the
  Hierarchies section with the node-tree model (replacing the `roots/collapsedTags/
  disabledTags` sketch).
- **[99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md)**: tick the Hierarchies item (resolver +
  CRUD); note WebDAV still carries the write endpoints.

```
