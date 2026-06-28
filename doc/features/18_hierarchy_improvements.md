# Hierarchy improvements

## 1. Overview & goals

Four refinements to the hierarchy model ([`05_hierarchies.md`](05_hierarchies.md)) and its
WebDAV write-back layer ([`06_webdav.md`](06_webdav.md)), tracked from the "New complex
features → Hierarchies" bullets in [`15_qol_improvements.md`](15_qol_improvements.md):

1. **Drop directory node** — a write-only sink that accepts any upload but lists nothing.
2. **Per-node write-back, tree-inherited** — write-back becomes a tri-state per-node control
   (`inherit | on | off`) under one hierarchy-wide master switch, configurable in each node's
   Advanced section.
3. **Write-back on untagged queries** — lift the hard read-only restriction on
   `matchUntagged` nodes.
4. **Mirror depth limit + foreign excludes** — cap how deep a `mirror` expands, and let a
   mirror's `exclude` list reference tags outside its `tagRoot`.

Core invariant is unchanged: **a hierarchy stores no pictures.** Every directory is a live
function of the tag graph; these changes only touch the `config` JSONB shape, its validation,
the read resolver's directory generation, and the write-back gating — **no schema columns**.

## 2. Decisions (settled)

- **`kind: "drop"` (§4).** A new explicit node kind: a leaf, always shown (exempt from
  empty-dir hiding), whose listing is **always empty** (no pictures, no children). Writes into
  it apply a **fixed, configurable `onAdd` op-list** (assign tags). It is the only node that is
  **writable even when the master switch is off** (§5.4).
- **Tri-state inherited write-back (§5).** Each node gains `writeBackEnabled: bool | null`
  (`null` = inherit). The hierarchy `writeBack` master switch is the **root default** *and* a
  **hard ceiling**: `writeBack: false` forces every non-drop node read-only regardless of
  per-node settings. With the master on, the **nearest explicit ancestor wins**. A `static`
  node isn't writable itself, but its toggle sets the inherited default for its descendants.
- **Untagged is writable (§6).** Drop the `matchUntagged ⇒ no writeBack` validation. Untagged
  nodes accept the same op-list editor, but the op-list is **free-form (not compliance-checked
  §7.2 of 05)** — "untagged" is not expressible as include/exclude, so structural validation
  can't apply. The frontend warns on `onAdd` that true untagged status can't be guaranteed if a
  pipeline tag remains.
- **`safeDeleteMode` coupled to write-back (§5.3).** A node's `safeDeleteMode` is only
  meaningful/configurable when the node is effectively write-back-enabled. A read-only node's
  delete is always `fullDelete` (no tag to single-branch-remove).
- **Mirror `maxDepth` + `deeperMode` (§7).** A `mirror` node gains `maxDepth` (`0`/absent =
  unrestricted) capping directory generation to N tag levels below `tagRoot`, and
  `deeperMode: "collapse" | "exclude"` (default `collapse`) deciding whether pictures below the
  cap roll up to the deepest allowed directory or disappear.
- **Foreign mirror excludes (§7.3).** A `mirror`'s `exclude` entries no longer must be
  `<@ tagRoot`. A **sub-tag** exclude prunes directories **and** pictures (today's behaviour); a
  **foreign** exclude (not under `tagRoot`) is a **pure picture-membership cut** — reject any
  picture carrying it, no directory effect. `collapsed` stays restricted to `<@ tagRoot`.
- **`version: 2`.** Bumped to signal the new shapes. v1 blobs deserialize forward unchanged
  (new fields default: `writeBackEnabled = null`, `maxDepth = 0`, `deeperMode = collapse`), so
  no migration/backfill is needed.

## 3. Config schema changes

`hierarchies.config` JSONB (`domain::hierarchy`). All additions are optional/defaulted.

### 3.1 Node — common fields (`+ writeBackEnabled`)

```jsonc
{
  "id": "n_ab12",
  "kind": "mirror | query | static | drop",
  "name": "Photos",
  "naming": null,
  "safeDeleteMode": null,
  "writeBackEnabled": null   // NEW. null = inherit nearest explicit ancestor (root = master
                             //   switch); true/false = override for this node + its subtree.
}
```

### 3.2 `kind: "drop"` (new)

```jsonc
{
  "id": "n_inbox", "kind": "drop", "name": "Inbox",
  "onAdd": [ { "op": "assign", "path": "Inbox" } ]   // fixed tags applied to every upload
  // no children, no read predicate, no `exclude`/`include`. Listing is always empty.
  // `writeBackEnabled` is ignored (always writable); `safeDeleteMode`/`naming` are irrelevant
  //   (nothing is ever listed, so nothing is read/moved/deleted out).
}
```

`onAdd` is a normal op-list (`assign`/`remove`), but in practice `assign` only; it is
**free-form, not compliance-checked** (there is no read predicate to satisfy).

### 3.3 `kind: "query"` (`matchUntagged` may now be writable)

Unchanged shape. The only change: `writeBack` is **permitted** alongside `matchUntagged: true`.
When `matchUntagged` is set, the op-list bypasses §7.2 compliance (free-form).

### 3.4 `kind: "mirror"` (`+ maxDepth`, `+ deeperMode`, relaxed `exclude`)

```jsonc
{
  "id": "n_photos", "kind": "mirror", "name": "Photos",
  "tagRoot": "Photos",
  "keepDir": false,
  "collapsed": ["Photos.Travel.Alps.Hiking"],   // still must be <@ tagRoot
  "exclude":   ["Photos.Outdoor", "Private"],    // NEW: entries may be foreign to tagRoot
  "maxDepth": 2,                                  // NEW. 0/absent = unrestricted. Counts tag
                                                  //   levels below tagRoot (keepDir-independent).
  "deeperMode": "collapse"                        // NEW. "collapse" (default) | "exclude".
}
```

## 4. Drop directory node

A `drop` node is a **write-only inbox**. Conceptually it is a `query` node with no read
predicate (lists nothing) and a fixed `onAdd`.

- **Read** (`tree`/`browse`/PROPFIND): the directory exists and is **always shown** (it is
  exempt from the §5.2-of-05 empty-directory hiding, since by design it has zero visible
  pictures). `browse` returns an empty page; `tree` reports `child_count: 0`,
  `picture_count: 0`, `writable: true`.
- **Write** (WebDAV `PUT`/`COPY`/`MOVE`-in): ingest/dedupe exactly like any write-back target
  (06_webdav.md §7–8), then apply `onAdd`. A dedupe hit (existing owned picture by hash, or an
  explicit MOVE/COPY source) just applies `onAdd` to that picture. `MKCOL` inside a drop node →
  `405` (it is a leaf). `DELETE`/`MOVE`-out can't occur (nothing is listed).
- **Always writable**, ignoring `writeBackEnabled` and the master switch (§5.4). It has no
  predicate to break, so it never raises the §7.2 `409`; it may still raise the live-service
  `409` if `onAdd` removes a tag a service re-asserts (it normally only assigns).
- **Validation:** `onAdd` paths parse as `TagPath` (SharedToMe allowed as elsewhere); no
  compliance check; `static`/`mirror` rules about children don't apply (drop is a leaf).

## 5. Per-node write-back (tri-state, inherited)

### 5.1 Effective-enabled resolution

```
effective_enabled(node):
  if not config.writeBack:        return false        # master = hard ceiling (non-drop)
  walk root → node, tracking the last Some(writeBackEnabled) seen (root seed = true)
  return that value
```

- The hierarchy `writeBack` master switch is **both** the root default (when on) **and** a hard
  global off-switch: with it off, every non-drop node is read-only and no per-node toggle can
  re-enable it.
- With the master on, `writeBackEnabled` is a tri-state: `null` inherits the nearest explicit
  ancestor (or the root's `true`); `true`/`false` overrides for the node **and its subtree**
  (until a deeper node overrides again).

### 5.2 Writability matrix (replaces §7.5 of 05)

| Node kind                        | Writable (add/copy/move-in/singleBranch delete)           | fullDelete |
|----------------------------------|-----------------------------------------------------------|------------|
| `drop`                           | **always** (ignores master + `writeBackEnabled`)          | n/a        |
| `mirror`                         | `effective_enabled` (implicit op-list)                    | allowed    |
| `query`, has op-list             | `effective_enabled`                                       | allowed    |
| `query`, no op-list              | read-only (`403`)                                         | allowed    |
| `query` `matchUntagged`, op-list | `effective_enabled` (free-form ops, §6)                   | allowed    |
| `static`                         | read-only (`403`) — toggle only sets descendants' default | n/a        |

`config.writeBack: false` ⇒ every row above except `drop` collapses to read-only. `fullDelete`
is still allowed everywhere (no tag mutation, never a `409`).

### 5.3 `safeDeleteMode` coupling

`safeDeleteMode` only matters when a node is effectively writable: `singleBranch` applies
`onRemove`, which requires write-back. On a read-only node a delete is always `fullDelete`. The
frontend therefore disables the `safeDeleteMode` control (and pins it to `fullDelete`) whenever
the node's effective write-back is off.

### 5.4 Master switch vs drop

The master switch is the hierarchy-wide kill switch for **predicate-backed** nodes. **Drop
nodes are the deliberate exception** — they remain writable even when `writeBack: false`, so an
"inbox-only, browse read-only" hierarchy is expressible. (This is the one relaxation of the
05/06 "`false` ⇒ entire hierarchy read-only" invariant; documented in §9.)

## 6. Write-back on untagged queries

- Validation no longer rejects `writeBack`/op-lists on `matchUntagged: true`.
- The op-list is **free-form**: the §7.2-of-05 compliance check (which is phrased over
  `include`/`exclude`) is **skipped** for untagged nodes, because "no stored tag of any source"
  is not an include/exclude predicate. The author may declare whatever `onAdd`/`onRemove` ops
  make sense (typically `onRemove: [assign <some tag>]` to "file a picture out of the inbox",
  and `onAdd: [remove <manual tags>]`).
- **Frontend warning:** when an untagged node has an `onAdd`, the editor shows a non-blocking
  warning that `onAdd` cannot guarantee the picture becomes untagged — a live `rule`/`segment`/
  `share_mapping` tag will keep it out of the directory after the write (and may raise the
  runtime `409`).
- Runtime `409` still applies: an `onRemove`/`onAdd` op that drops a tag a live service
  re-asserts fails as in 06_webdav.md §7.2.

## 7. Mirror depth limit & foreign excludes

### 7.1 `maxDepth`

Caps directory generation under `tagRoot` to `maxDepth` **tag levels below `tagRoot`**
(`tagRoot`'s direct children = level 1). `maxDepth = 0`/absent = unrestricted. `keepDir` is
independent: it only decides whether `tagRoot` itself is rendered as a wrapping level; it does
**not** consume the depth budget. Applies **on top of** explicit `collapsed`/`exclude`.

Validation: if present, `maxDepth ≥ 1`.

### 7.2 `deeperMode`

Governs pictures whose deepest tag sits below the `maxDepth` cut (and isn't already handled by
an explicit `collapsed`/`exclude`):

- **`collapse`** (default): the picture rolls up to the **deepest allowed** (level-`maxDepth`)
  ancestor directory — the existing collapse roll-up, applied by depth rather than by an
  explicit list. No directory is generated below the cut.
- **`exclude`**: the picture disappears from the mirror (like an `exclude` subtree), and no
  directory is generated below the cut.

Resolver: directory generation stops at `maxDepth`; the per-directory predicate at the cut
either OR-folds the deeper subtree (`collapse`, an `<@` arm like a collapsed root) or adds a
deeper-subtree `exclude` cut (`exclude`).

### 7.3 Foreign excludes

`exclude` entries are no longer required to be `<@ tagRoot`:

- **Sub-tag exclude** (`<@ tagRoot`): unchanged — prune the subtree's **directories and
  pictures** (§5.4-of-05).
- **Foreign exclude** (not under `tagRoot`): a **picture-membership cut only** — every
  directory in this mirror additionally rejects pictures carrying that tag (an inclusive
  `NOT EXISTS … <@ foreign` arm). No directory is affected (the tag isn't in the subtree).

This lets a mirror say "expand `Photos`, but never surface anything also tagged `Private`",
where `Private` is a sibling root, not under `Photos`.

Validation: `collapsed[i] <@ tagRoot` still required; `exclude[i]` only needs to parse as a
`TagPath`.

## 8. Implementation

### 8.1 Domain (`domain/hierarchy.rs`)

- `NodeKind::Drop { on_add: Vec<TagOp> }` (or `WriteBack` with only `on_add` used).
- `Node.write_back_enabled: Option<bool>` (`writeBackEnabled`).
- `NodeKind::Mirror { …, max_depth: u32 (default 0), deeper_mode: DeeperMode (default Collapse) }`.
- `enum DeeperMode { Collapse, Exclude }` (`serde rename_all = "lowercase"`).
- `HierarchyConfig.version` default bumps to `2`.
- **Validation:**
    - `drop`: `on_add` paths parse; leaf (no children); no compliance check.
    - `query` + `matchUntagged` + `writeBack`: **allowed**, skip compliance.
    - `mirror`: `collapsed[i] <@ tagRoot`; `exclude[i]` parse only; `maxDepth ≥ 1` if present.
    - effective-enabled is computed during validation only where it gates a structural error
      (e.g. a `query` that is effectively enabled should carry an op-list — otherwise it silently
      falls to read-only, which is acceptable, so this stays a soft/UI concern, not a hard error).
- **`effective_enabled`** helper (walks ancestor `writeBackEnabled`, root seed = `config.writeBack`),
  used by the resolver and the VFS writability projection.

### 8.2 Resolver (`services/hierarchy.rs`)

- `build_tree`: emit `drop` directories (always, never hidden); stop mirror expansion at
  `maxDepth`; apply `deeperMode` at the cut.
- `predicate_for_path`:
    - drop dir → an "empty" predicate (matches nothing) for reads.
    - mirror dir at the cut → OR-fold deeper subtree (`collapse`) or add deeper `exclude` arm.
    - foreign excludes → extra `exclude` arms on every mirror directory predicate.
- Writability projection (fed to `VfsEntry.writable`): use `effective_enabled` + the §5.2
  matrix; drop is always writable.

### 8.3 VFS / WebDAV (`services/vfs.rs`, `api/webdav.rs`)

- `list_dir`/`stat` on a drop node → empty children, `is_dir`, `writable = true`.
- `write` into a drop node → ingest/dedupe (existing §7–8 of 06), apply `onAdd`. `MKCOL`
  inside drop → `405`.
- Master-switch gate: keep "whole hierarchy read-only when `writeBack: false`" for predicate
  nodes, but **exempt drop** nodes.
- Untagged write-back path reuses the existing op-list apply (no compliance gate).

### 8.4 Frontend (`components/hierarchies/`)

- `NodeEditor`: add the **Drop** kind (name + `onAdd` `TagListField`); per-node
  **write-back tri-state** control (`inherit | on | off`) in the Advanced disclosure for every
  kind — for `static` it sets only the descendants' inherited default (with the existing hover
  message that static can't be written into); for `drop` it is shown **on, disabled**; gated to
  `off`/disabled when the master switch is off.
- `WriteBackEditor`: enable on `matchUntagged` query nodes with the §6 `onAdd` warning.
- `MirrorEditor`: `maxDepth` `NumberInput` (0 = unrestricted) + `deeperMode` switch
  (collapse/exclude) shown only when `maxDepth ≥ 1`; allow **foreign** tags in the `exclude`
  `TagListField` (don't restrict to `tagRoot` descendants there; keep `collapsed` restricted).
- Gate the `safeDeleteMode` control on effective write-back (§5.3).

## 9. Edge cases

- **Drop under master-off** is the one place a hierarchy can be writable while the master switch
  is off (§5.4) — by design (an inbox on an otherwise read-only mirror view).
- **`writeBackEnabled` on a `mirror`/`static`** with the master off is inert (the ceiling wins);
  the editor disables it.
- **`maxDepth` + explicit `collapsed`/`exclude`**: both apply; a tag pruned by an explicit
  `exclude` never reaches the `deeperMode` logic.
- **Foreign exclude == `tagRoot` subtree by coincidence**: if a foreign exclude path happens to
  be `<@ tagRoot`, it is treated as a sub-tag exclude (prunes dirs) — classification is by
  prefix, not by author intent.
- **Untagged `onAdd` no-op**: a write into an untagged dir whose `onAdd` can't make the picture
  untagged (pipeline tag survives) succeeds as a tag mutation but the picture won't re-appear
  there — surfaced by the §6 warning, not an error.

## 10. Testing

- **domain:** drop validation (leaf, `onAdd` parse); `writeBackEnabled` tri-state
  `effective_enabled` resolution (master ceiling, nearest-ancestor-wins, static-propagates,
  drop-exempt); `matchUntagged` + `writeBack` now valid (compliance skipped); mirror `maxDepth`
  bound + `deeperMode`; foreign vs sub-tag `exclude` classification.
- **repository (`push_filters`):** depth `collapse` OR-fold and `exclude` cut; foreign-exclude
  `NOT EXISTS <@` arm.
- **services (resolver):** drop dir always shown/empty; mirror expansion stops at `maxDepth`;
  collapse vs exclude at the cut; writability projection per the §5.2 matrix.
- **vfs/webdav:** PUT into a drop node (new + dedupe) applies `onAdd`; drop writable with master
  off; `MKCOL` inside drop → `405`; untagged write-back apply.

## 11. Documentation to update

- **[05_hierarchies.md](05_hierarchies.md):** the `drop` kind, `writeBackEnabled`,
  `matchUntagged` writability, mirror `maxDepth`/`deeperMode`/foreign-exclude; the revised
  writability matrix (§5.2 here supersedes its §7.5); `version: 2`.
- **[06_webdav.md](06_webdav.md):** drop-node writes; drop exempt from the master read-only
  ceiling; untagged write-back.
- **[01_GENERAL_SPECIFICATIONS.md](../01_GENERAL_SPECIFICATIONS.md) §4:** mention drop nodes,
  per-node inherited write-back, mirror depth limit.
- **[03_BACKEND_ARCHITECTURE.md](../03_BACKEND_ARCHITECTURE.md):** note the resolver/VFS changes
  if they affect the module summary.
- **[06_API_REFERENCE.md](../06_API_REFERENCE.md):** any `tree`/`browse` response notes for
  drop dirs (always-shown, empty).
- **[99_ROADMAP_MVP.md](../99_ROADMAP_MVP.md)** and
  **[15_qol_improvements.md](15_qol_improvements.md):** tick these four bullets, pointing here.
