# Feature 39: Hierarchy write-back & projection

## 0. Status

Implemented (backend, frontend, tests). Deviations and the choices the spec left open are in §15;
flat `query` nodes (§5.5) stay recorded, not built, and the §8 verification hypothesis is still to be
confirmed from an access log.

## 1. Overview & goals

Three complaints, two of which share a cause:

- **Write-back fires the wrong ops.** A WebDAV MOVE applies the destination's `onAdd` and the
  source's `onRemove` independently, with no notion of what the picture is actually *leaving*. Moving
  between two directories under the same `query` node re-applies that node's ops pointlessly.
- **Manual tags shadow service tags forever.** 05 §7.2 requires a writable node's op-list to assign
  every `include` arm, and write-back only writes `manual` rows (05 §7.3) — so moving a file into a
  mirror directory whose tag a **segmentation service** produces assigns that tag manually. Service
  cleanup never touches `manual` rows (01 §3.5), so the shadow outlives the service and survives a
  band re-definition.
- **A mirror cannot project.** `maxDepth` (18 §7.1) caps directory generation and rolls pictures up;
  nothing skips the *first* N levels, so `Era.<era>.<travel>` can never render as `<travel>/`
  directories. With no mirror-scoped `include` either, a second flatter view of the same events means
  duplicating the tags — stranding them with no `tag_metadata` (34 §3) and doubling every count,
  share target and rename.
- **Sync clients re-upload.** A `drop` node lists nothing by design (18 §4), so a client that
  verifies a `PUT` by re-reading the resource never succeeds and retries indefinitely; and a
  byte-identical re-upload of a **trashed** picture currently un-deletes and re-tags it
  (`vfs/write.rs:198`), so deleted photos come back.

The fix for the first is a **transition model** (§3). The fix for the second is **not** in write-back
at all — it is a pipeline-side prune (§4), which converges on the minimal manual anchor without the
write path having to predict anything. The third is two new `mirror` controls (§5); the fourth is §6–8.

## 2. Decisions (settled)

- **Write-back is a transition, not two independent events.** One rule — the set difference between
  the source's and destination's node chains (§3.1) — replaces per-verb special cases and makes the
  sibling-move case fall out instead of being written down.
- **Shadows are removed after the fact, not avoided up front.** Write-back keeps doing the dumb,
  synchronous thing: assign the directory's own tag as `manual`, so the file appears where it was
  dropped immediately. The next pipeline run notices the service asserts the same path and prunes the
  redundant manual row, reducing it to the minimal anchor the gates actually need (§4).
  An earlier draft inverted this — an inline per-picture pipeline run at write time, deciding which
  arms to skip — and it was rejected (§2.1).
- **`manual` tags are no longer untouchable.** This is the one real invariant change in the feature.
  01 §3.5's "`manual` and `incoming_share` tags are never touched" becomes: `incoming_share` never,
  `manual` only by an opted-in service, only when that service asserts the identical path, and never
  below the anchors the gates require (§4.4). It is opt-in per service and `DEFAULT false`, so nothing
  changes until a user asks for it.
- **Anchors are derived, not declared.** The tags that must survive a prune are read out of the
  `requires`/`excludes` arms of the services that actually fired for that picture (§4.3–4.4), not from
  a list the user maintains. A user keep-list exists (§4.5) but merges into the same set as extra
  anchors rather than being a parallel mechanism.
- **`mirror.exclude` keeps both of its meanings.** The sub-tag case prunes directories and pictures
  (05 §5.4); the foreign case is a pure picture-membership cut (18 §7.3). They are not
  interchangeable and neither is redundant (§5.4).
- **Trashed duplicates are a policy, not a node-kind rule.** `trashedDuplicate` (§6) is a tri-state
  resolved like `writeBackEnabled` (18 §5.1), with `drop` nodes created carrying an explicit value so
  the behaviour is visible in the editor rather than hidden in a resolver branch.

### 2.1 Rejected / deferred

- **`preferServiceTags` and the inline per-picture pipeline run — rejected.** The earlier shape: at
  write time, apply the removes, run the pipeline inline for that one picture, then assign only the
  `onAdd` arms not already satisfied, 409ing if any arm proved unsatisfiable. It reached the same end
  state as §4 but required extracting a per-picture entry point out of
  `routines::pipeline::evaluation::run_for_user` (~150 lines), a per-user parsed-`ServiceConfig` cache
  to keep bulk MOVEs from reloading the service list per file, an arm-by-arm `direct`-predicate check,
  and a new add-side 409 — all to make convergence synchronous. §4 gets there on the next debounced
  run with none of it, and async convergence is the house model anyway (34 §4: *"the tag tree is
  already eventually consistent"*). It also had a dead spot: the only configuration that produced a
  shadow under `on` was a gate depending on the tag family being entered, which is exactly where
  skipping was unsafe.
- **A user-initiated "redundant manual tags" maintenance check — rejected.** §4.6 makes it
  unnecessary: toggling the option invalidates the service, which dirties the user's pictures, which
  prunes every pre-existing shadow on the next run.
- **Hierarchy-wide `config.exclude` / `config.include` — rejected.** A `config`-level arm is seeded
  into *every* directory's predicate, including the node you would use to reach the excluded
  pictures. That breaks the only pattern anyone wants it for — a mirror with a subtree hidden plus a
  `query` beside it to reach it all at once — which **already works** via a foreign `mirror.exclude`
  (18 §7.3). `config.include` is worse: nothing at config level carries an op-list, so a `PUT` could
  never satisfy the global arm and the file would land invisible.
- **Flat `query` nodes (`keepDir: false`) — deferred, specced as the upgrade path (§5.5).** The right
  primitive for *branch-scoped* filtering, but nothing needs it yet: both live cases are mirror-local.
- **WebDAV filtering of sensitive pictures — not here.** Feature 38 §2.1 postponed it to its own
  feature, and 38 §2 is careful that the **hierarchy resolver never reads `tag_metadata`**. A
  `config.hideSensitive` would make it a third exception to 34 §2 and deserves its own doc. §5.4
  records why the hand-rolled stopgap is stronger than 38 §2.1 originally claimed.

## 3. Write-back as a transition

### 3.1 The difference rule

For a write with a source directory and/or a destination directory, let `A_src` and `A_dst` be the
**node chains** (self + ancestors) of each.

- Apply `onRemove` for every node in **`A_src \ A_dst`** — the nodes being left.
- Apply `onAdd` for every node in **`A_dst \ A_src`** — the nodes being entered.
- A node in both fires nothing. The picture never left it.

A move between two mirror directories under the same `query` keeps that query in both sets, so its
`Travel` assign/remove does not fire. A move out of the subtree puts it in `A_src` only, so its
`onRemove` does. Today's "apply the destination's ops and the source's ops" is the degenerate case
where the chains share nothing.

This also retires the idea that each node's op-list must satisfy its *effective*
(ancestor-conjoined) predicate. It need not: 05 §11 keeps validating each node against **its own**
predicate, and the conjunction is satisfied at runtime by composing the op-lists of the nodes
entered. A `query` assigning `Travel` and a mirror assigning `Era.2026.Vietnam` together satisfy both
arms, and neither node ever learns about the other.

### 3.2 Verb mapping

| Verb | `A_src` | `A_dst` | Ops |
|---|---|---|---|
| `PUT` (new or dedupe hit) | ∅ | destination chain | every `onAdd` in `A_dst` |
| `COPY` | source chain | destination chain | `onAdd` for `A_dst \ A_src`; no removes |
| `MOVE` | source chain | destination chain | the full difference (§3.1) |
| `DELETE`, `singleBranch` | source chain | ∅ | **deepest node only** |
| `DELETE`, `fullDelete` | — | — | **no tag ops at all** (05 §7.4) |

`singleBranch` DELETE is deliberately *not* the difference rule. Its whole purpose is to make a dumb
client's delete minimal (05 §7.4), and an ancestor tag is by definition not a single branch —
stripping `Travel` because a file was deleted from `Voyage/Era/2026/Vietnam/` would be the opposite of
the guarantee. `fullDelete` already mutates no tags, so it is unaffected.

### 3.3 Applying the ops

One transaction, removes before adds, `manual` rows only — unchanged from today except for *which*
nodes contribute. There is no membership check, no inline pipeline run and no add-side 409.

The practical consequence is that the file appears in the destination immediately, because the
directory's own tag is written synchronously. The pre-existing quirk is also unchanged: a picture
carrying a deeper tag is not a *direct* file of the parent directory (05 §5.3's `minus_children`), so
dropping it into `Era/2026/` while it holds `Era.2026.Vietnam` still shows it under `Vietnam/`. A
genuine MOVE out of `Vietnam/` clears that tag via §3.1 and the case does not arise.

Convergence to the minimal manual tag happens on the next debounced pipeline run (§4), not here.

## 4. Shadowed manual tag pruning

A pipeline mechanism, specced here because it is what lets §3.3 stay three sentences long.

### 4.1 The option

Per service, uniform across service types, so it is a column rather than per-type `config` JSONB:

```sql
ALTER TABLE tagging_services
    ADD COLUMN prune_shadowed_manual_tags boolean NOT NULL DEFAULT false,
    ADD COLUMN keep_manual_tags           ltree[] NOT NULL DEFAULT '{}';
```

`DEFAULT false` so no existing service changes behaviour. The create-service dialog pre-checks it;
the editor copy is *"Remove manual tags this service also produces — except at or above: […]"*.

### 4.2 Candidates

A **candidate** `M` is a `manual` tag row on picture `P` whose exact path was produced, in this run,
by a service with the option on. Exact path only — an ancestor or descendant of a produced tag is not
a shadow. `incoming_share` rows are never candidates.

Note what pruning does and does not do: it removes a redundant **row**, not the tag. After the prune
the picture still carries the path, asserted by the service. The only way a tag is genuinely lost is
the feedback loop — prune the row, the producing service's own gate depended on it, the service stops
producing, and the reconcile removes its row too. §4.4 exists for that single failure mode.

### 4.3 Gate history

The evaluator is already `for picture in chunk { for service in &services { … } }`, and the gate
decision is computed there regardless, so recording it costs nothing and no extra pass. `requires`
and `excludes` are plain `ltree[]` columns on `tagging_services` (`schema.sql:475`), loaded with the
service list and in memory for the whole run.

Per picture, per service, record:

- whether the service **fired**;
- for each satisfied `requires` arm, which tags in the gating set satisfied it (presence is inclusive
  against virtual ancestors, 01 §3 — a tag `T` satisfies arm `R` when `R @> T`);
- if it did not fire: which `excludes` arms matched and by which tags, and whether any `requires` arm
  failed **independently** of the candidate.

That last field is what makes §4.4's rows 3 and 5 decidable — knowing *why* a service did not fire,
not merely that it did not.

### 4.4 Anchors

`requires` is an AND (01 §3: fires only with **all** of `requires` and **none** of `excludes`), so
breaking a single arm stops the service. The rule is therefore per-arm, and one sole-satisfied arm is
enough to pin a candidate.

| Fired? | Condition | Keep `M`? |
|---|---|---|
| yes | `M` is the **sole** satisfier of ≥1 `requires` arm `R` | **yes** — anchor at `deepest(R)` over those arms |
| yes | every arm `M` satisfies has another satisfier | no |
| no | failed some `requires` arm independently of `M` | no — removing `M` only subtracts |
| no | the **only** blocker was an `excludes` arm matched by `M` alone | **yes**, unreduced (below) |
| no | blocked by that exclude **and** a failing `requires` arm | no — still won't fire |

> `anchors(M)` = { `requires` arms `M` alone carries, over services that fired }
> ∪ { `keep_manual_tags` entries that are ancestor-or-self of `M` }
>
> - empty → prune `M`
> - otherwise → assign manual `deepest(anchors(M))`, then prune `M` (a no-op when it equals `M`)

`deepest()` because the arms are all ancestors-or-self of `M` and presence is inclusive, so the
deepest one satisfies every shallower arm with a single row. No search over ancestors is needed: the
candidate anchors *are* the arms, read straight out of the service that gates on them.

**The exclude row is kept unreduced, deliberately.** For a `requires` arm the anchor is a tag the
user implicitly wanted — it is why the service fired. For an `excludes` arm, assigning an ancestor to
preserve an exclusion would invent an assertion the user never made, and would exclude the picture
more broadly than before. Keeping `M` as-is is both safe and honest.

Worked example — the case that motivated the whole thing. Segmentation gated `requires: [Era]`
produces `Era.2026`; the picture holds a manual `Era.2026` written by write-back. The service fired,
and `Era.2026` is the sole satisfier of the `Era` arm. So: assign manual `Era`, prune manual
`Era.2026`. End state is manual `Era` + segment `Era.2026` — the minimal anchor, with the derived
level owned by the service, stable across runs.

### 4.5 Keep-list

`keep_manual_tags` entries merge into `anchors(M)` as extra arms, so there is one code path and one
mental model: a floor below which a manual tag is not reduced. Listing `Era` means "always keep me at
least `Era`"; listing `Era.2026` means "do not reduce me past `Era.2026`". If a gate derives `Era` and
the user listed `Era.2026`, `deepest()` picks `Era.2026` — the safe direction.

It exists for the two things the derived set genuinely cannot infer: a pruned tag is no longer
hand-removable (`PATCH /tags` only removes `manual` rows), and it no longer survives a disable (§4.7).
Both are per-tag judgements, not properties of the config.

The editor shows the derived anchors read-only beside the list, so a user can see what is already
being kept for them and does not duplicate it.

### 4.6 Timing and backfill

Pruning runs per picture after every service has been evaluated — the gate history must be complete —
in the same transaction as `PipelineRepository::reconcile_pipeline_tags`. No re-dirty is needed: §4.4
guarantees the next run's gate decisions are unchanged.

Backfill is free. `update_service` already calls `TaggingServiceRepository::touch_invalidated`, and
`find_dirty_for_user` treats a picture as dirty when any enabled service's `last_invalidated_at` is
newer than its `last_pipeline_run_at` (`repository/pipeline.rs:115`). So toggling the option
invalidates the service, dirties the user's pictures, and prunes every pre-existing shadow on the next
run — no migration-time backfill, no one-off statement, no maintenance pass.

### 4.7 Disabling a service

`update_service` removes a service's tags unconditionally when it is disabled
(`services/tagging.rs:31`), with no promotion option — that exists only on delete
(`DELETE /tagging-services/{id}?promote_tags=`). With pruning on, the manual shadow is gone, so
disabling drops the tag from every covered picture until the service is re-enabled.

Accepted: a service's output is a pure function of picture data plus its config, so re-enabling
re-derives the identical tags, and `remove_service_tags` already resets `last_pipeline_run_at = NULL`
so they come back on the next run.

Two consequences to handle in the UI rather than the engine:

- **Disable stops being a switch.** It becomes a menu action with a confirmation dialog carrying a
  *"promote tags to manual"* checkbox, mirroring the delete flow. The checkbox defaults **checked when
  the service has pruning enabled**, unchecked otherwise. Services are not toggled often enough for a
  dialog to be a burden.
- **Share announcement churn.** While the service is disabled, pictures whose only remaining path to
  a share was its tag drop out of coverage, so the pipeline unannounces them and re-announces on
  re-enable — visible to recipients. Already true for pictures with no manual shadow; pruning widens
  the set. Worth a line in the dialog.

## 5. Mirror projection

### 5.1 `include`

```jsonc
{ "kind": "mirror", "tagRoot": "Era", "include": ["Travel"] }
```

A **foreign include** arm: every directory in this mirror additionally *requires* the picture to carry
that tag — the symmetric counterpart of 18 §7.3's foreign exclude, and scoped to the one mirror, which
is what `config.include` could never be. No directory effect of its own; combined with §5.3 the
directories that end up empty disappear.

Entries need only parse as a `TagPath`; an entry `<@ tagRoot` is pointless but harmless (every picture
in the mirror already carries it) and is not rejected.

The mirror's **implicit op-list is extended** to assign the `include` arms alongside the directory's
own tag, so a write into the projection satisfies it.

### 5.2 `minDepth`

Caps directory generation from the *top*: `minDepth: N` makes level `N` below `tagRoot` (its direct
children = level 1, as for `maxDepth`) the shallowest generated level, suppressing levels `1…N−1`
and hoisting level `N` to the mirror node's level. `0`/`1`/absent = today's behaviour. Independent
of `keepDir`, which only decides whether `tagRoot` itself is rendered (as a read-only container
above the hoisted level), and composable with `maxDepth` (18 §7.1) — the two bound one range, so
`minDepth: 2, maxDepth: 2` projects exactly one level.

So `Era.<era>.<travel>` with `minDepth: 2` renders `<travel>/` directories, with no duplicated tags
and the metadata still single-homed on the real `Era.<era>.<travel>` tag (34 §3).

Pictures whose deepest tag sits **above** the cut have no directory to live in and are dropped from
this mirror — the projection is a view of level `N+1` and below, not of everything. There is
deliberately no `shallowerMode` counterpart to `deeperMode`: rolling a picture *down* into an
arbitrary descendant has no sensible choice of target.

Validation: `minDepth ≥ 0`; when both are set, `minDepth ≤ maxDepth`.

### 5.3 Precise empty-directory pruning

05 §5.2 already hides empty directories, but notes that the `tree` endpoint computes this "from path
presence" and that "precise zero-picture pruning may be deferred to the `counts=true` path to stay
cheap". Path presence is no longer sufficient: a foreign `include` (§5.1) filters *pictures*, so
without precise pruning a mirror over `Era` with `include: ["Travel"]` generates a directory per event
and leaves every non-travel one visibly empty.

Make pruning precise for a mirror carrying `include` or a foreign `exclude`: a directory whose entire
subtree resolves to zero visible pictures is omitted, computed from the per-directory predicate rather
than the path set. Unfiltered mirrors keep the cheap path-presence test.

### 5.4 `exclude` keeps both meanings

| Entry | Semantics | Replaceable? |
|---|---|---|
| `<@ tagRoot` (sub-tag) | Prune the subtree's **directories and pictures**. A picture with a tag outside `T` still shows under that other directory (05 §5.4). | No — a path-set prune, not a membership cut. |
| Not `<@ tagRoot` (foreign) | Pure **picture-membership cut**: reject any picture carrying it, from every directory in this mirror. No directory effect (18 §7.3). | No — mirror-scoped, which no `config`-level arm can be. |

Both stay, with the `<@ tagRoot` discriminator documented at the field rather than split into two
fields — a split would need a config migration for a readability gain.

**Correcting 38 §2.1.** It originally said a hierarchy can exclude a flagged tag by hand "with the
05 §5.4 caveat that a picture also tagged outside it still surfaces there". That caveat applies only
to the **sub-tag** case. For a tag *foreign* to `tagRoot` — the common shape, a `/Sensitive` or
`/Photos/Medical` root sitting beside `Era` — the foreign cut rejects the picture from the whole
mirror regardless of its other tags, so the stopgap is materially stronger than 38 claimed.

### 5.5 Upgrade path: flat `query` nodes

Recorded, not built. A `query` with `keepDir: false` contributes its predicate to its children and
emits no directory of its own — the same concept `mirror.keepDir: false` already spells, and the right
primitive for *branch-scoped* filtering.

Measured against the current resolver it is small: `build_nodes` already threads ancestor predicates
(`resolver.rs:160`, `child_and.push(own_base)`), so the splice is `out.extend(child_dirs)` in place of
`out.push(…)`. Three further pieces: fold the flat node's `own_base` into each spliced child's
`own_for_parent` or the parent **over-excludes** (`resolver.rs:179`); teach `first_hoisted_mirror`
(`resolver.rs:91`) to look through flat nodes or MKCOL stops resolving; and decide that a flat node
**contributes no direct pictures** — a filter layer is not a place — which is a real behaviour change
from a non-flat `query`.

The cost is the frontend (§9): flat nodes make the authored tree and the resolved tree different
shapes for the first time. Nothing in this feature assumes a filter must be mirror-local, so it stays
strictly additive.

Rejected alongside it: **decomposing `query` into a filter layer plus a `static` dir.** Conceptually
the cleanest of the three, but a pure respelling with no capability gain, needing a config migration
of every existing `query` node, making the common case two nodes instead of one, and putting
`writeBack` on a non-directory — so a folder's writability would be decided by an invisible ancestor.

## 6. Trashed duplicates

`vfs::finalize_write` handles every node kind identically, and on a `file_hash` hit against a
**trashed** owned picture it un-deletes and re-tags (`vfs/write.rs:198`). That is deliberate — the
comment names the case: a client that renames by DELETE + `PUT` under a new name ("naive rename under
`fullDelete`"). It is also why photos deleted in the webapp reappear after the next phone sync.

Both behaviours are wanted, by different nodes, so it becomes a policy:

**`trashedDuplicate: inherit | restore | ignore`** — what to do when a `PUT`'s bytes match a trashed
owned picture. Resolved like `writeBackEnabled` (18 §5.1), **root seed `restore`**, so today's
behaviour and the naive-rename case are preserved.

- **`restore`** — clear `deleted_at` and apply `onAdd`.
- **`ignore`** — leave the picture trashed, apply no ops, return success. The client is satisfied and
  the delete stands.

A `drop` node is **created with `"ignore"` written explicitly into its config**, not `inherit`, so the
behaviour is visible and flippable in the editor rather than hidden in a resolver branch. The
rationale: a drop node's writes are machine-driven, so byte-identity is an accident rather than
intent, and a naive rename cannot occur where nothing is listed.

**Not a policy — a hard rule.** `restore` must clear `deleted_at`. Tag-without-restore produces a
picture that is tagged but excluded from every listing (05 §14, `deleted_at IS NULL`): invisible where
it was dropped, and a permanent re-upload magnet. Restore and tag, or do neither.

## 7. Drop-node verification listing

A `drop` node lists nothing by design (18 §4). Many WebDAV clients verify a `PUT` by re-reading the
resource, so that read fails every time and the client retries indefinitely — the likely cause of the
original re-upload loop (§8).

A drop node therefore lists the files it received within the last **N minutes** (default 10, runtime
setting `WEBDAV_DROP_VERIFY_WINDOW_SECS`), keyed `webdav:recentdrop:{hierarchy_id}:{node_id}`, name →
picture id.

This is **not** the `WebdavPendingDir` pattern removed in 34 §8.1. That sidecar asserted a directory
that did not exist; this one reports a file that does — the node simply chooses not to list it
permanently. And it is self-healing: if the window expires before verification, the retry re-`PUT`s,
dedupes on `file_hash`, and re-opens it.

The entry is listing-only. `GET` on it serves the picture normally (it is a real picture);
`DELETE`/`MOVE`-out within the window act on the picture, as in any other directory. The window does
not make the node accumulate — it is a verification affordance, not an inbox view.

## 8. WebDAV client compatibility

A short note for the docs, with one hypothesis flagged as **unconfirmed**.

- FolderSync keeps its **own per-pair sync state**, so a file it has already uploaded is not re-sent
  merely because it vanished from the remote listing. Evidence: removing a `Camera` tag from a
  picture — which removes it from the synced directory — did not trigger a re-upload.
- Which makes **post-write verification** the likely cause of the drop-node loop, not listing
  comparison. The signature is a `PROPFIND` or `GET` immediately following each `PUT` in the WebDAV
  access log; confirm there before relying on §7.
- A client that verifies writes cannot be satisfied by a node that lists nothing. Delete-after-upload
  may not rescue it either, since a client that cannot verify a transfer will generally not delete the
  source.

### 8.1 Android recipe

The configuration that works, as a first-class preset in the editor rather than something to
rediscover:

- A `query` node over a **permanent** provenance tag (`Camera`), whose `onAdd` assigns it. The
  directory lists its contents, so verification succeeds and the client reconciles.
- **`safeDeleteMode: singleBranch`** on that node, so a delete from the camera directory by Finder or
  a sync client means "remove from this directory" and not `fullDelete` → trash the picture.
- Moving pictures out later works by either route: untag in the webapp, or a WebDAV MOVE out of the
  directory, which fires that node's `onRemove` under §3.1.
- Splitting provenance (`Camera`, permanent) from triage state (`Inbox`, removable) is optional
  semantics, not a sync requirement.

A `drop` node with delete-after-upload is the other workable shape, at the cost of removing the photo
from the phone's own gallery; with §7 it no longer needs the client to delete.

## 9. Frontend

**Hierarchy editor — master-detail.** `NodeEditor.tsx` is 538 lines before this lands and the feature
adds three node fields. Restructure `HierarchyEditor` as master-detail: the authored node tree on the
left, the selected node's settings on the right.

Alongside it, show the **resolved directory tree** next to the authored one, reusing
`HierarchyDirTree.tsx` and the existing `tree` endpoint (05 §9.2). With `minDepth` and a foreign
`include` the two already differ — and if flat queries land (§5.5) an authored row can produce no
directory at all, at which point a single tree view would actively mislead. A flat node would then
render as a **wrapper** around its children with its filter as a chip, not as a sibling row.

New node controls: `mirror.include` (a `TagListField`), `mirror.minDepth`, `trashedDuplicate` (a
tri-state select beside `writeBackEnabled`), plus the §8.1 preset in `CreateHierarchyDialog`.

**Tagging-service editor.** `prune_shadowed_manual_tags` as a checkbox with the §4.1 copy,
`keep_manual_tags` as a `TagListField` beneath it, and the derived anchors rendered read-only beside
the list (§4.5). Replace the enable/disable switch with a menu action plus the §4.7 confirmation
dialog.

## 10. Config & schema changes

```jsonc
// hierarchies.config — version: 3
// Node — common fields
{ "trashedDuplicate": null }    // "restore" | "ignore" | null (inherit) — §6

// kind: "mirror"
{ "include":  [],               // foreign include arms — §5.1
  "minDepth": 0 }               // §5.2

// kind: "drop" — created with an explicit value
{ "trashedDuplicate": "ignore" }
```

`hierarchies.config` is JSONB, so v2 blobs deserialize forward unchanged (`include` empty, `minDepth`
0, `trashedDuplicate` null). `version` 2 → 3 signals the new shapes, per 18 §2.

One migration, `0023_prune_shadowed_manual_tags`, for the two `tagging_services` columns in §4.1.
38's `0022_sensitive_pictures` is the current head.

## 11. Validation

Additions to 05 §11:

- `mirror.minDepth ≥ 0`; when `maxDepth` is also set, `minDepth ≤ maxDepth`.
- `mirror.include[i]` parses as a `TagPath` (`SharedToMe` allowed, as elsewhere). No `<@ tagRoot`
  requirement — a foreign arm is the point.
- `mirror.exclude[i]` parses as a `TagPath`; the `<@ tagRoot` discriminator selects the semantics
  (§5.4) and is not a constraint. `collapsed[i] <@ tagRoot` is unchanged.
- `trashedDuplicate ∈ {restore, ignore}` when present.
- **05 §7.2 compliance is unchanged.** Nothing here alters what an op-list must be structurally
  capable of; §4 operates after the fact, on the pipeline side.

Service-side (`domain::tagging`):

- `keep_manual_tags[i]` parses as a `TagPath`. Reserved prefixes permitted — pruning never touches
  `incoming_share` rows anyway, so a `SharedToMe` entry is inert rather than wrong.
- `keep_manual_tags` is only meaningful with `prune_shadowed_manual_tags = true`; a non-empty list
  with the flag off is accepted and inert (so toggling the flag off and on does not lose the list).

## 12. Edge cases

1. **Sibling move under a shared `query`.** That node is in both chains, so `Travel` is neither
   removed nor re-assigned (§3.1). Previously both fired.
2. **MOVE out of the hierarchy's covered set entirely.** `A_dst = ∅` for the authored chain; every
   `onRemove` in `A_src` fires. This is a MOVE, not a `singleBranch` DELETE, so the chain rule
   applies.
3. **A candidate pinned by two gates at different depths.** S1 requires `Era`, S2 requires
   `Era.2026`, both fired, `M = Era.2026.Vietnam` is the sole satisfier of both. `deepest()` picks
   `Era.2026`, which satisfies `Era` inclusively. One row, both gates intact (§4.4).
4. **A candidate that is its own anchor.** `anchors(M) = {M}` — `deepest()` returns `M` and the prune
   is a no-op. The row stays `manual` and nothing is written.
5. **A service that did not fire because of a failing `requires` arm.** Its arms impose nothing: the
   candidate is pruned freely, because removing a tag can only subtract from an AND gate (§4.4 row 3).
6. **A candidate that is the sole blocker of an otherwise-firing service.** Kept, unreduced —
   assigning an ancestor to preserve an exclusion would invent an assertion the user never made
   (§4.4).
7. **Pruning and `assign_inner`'s ancestor rule.** Writing the anchor and deleting the candidate must
   be one statement: `assign_inner` prunes redundant manual *ancestors*, so a later re-assignment of
   the candidate path would silently delete the anchor just written.
8. **A disabled service with pruned shadows.** Its tags vanish until re-enable (§4.7). Re-enabling
   re-derives them identically; the §4.7 dialog offers promotion for the deliberate case.
9. **A picture above the `minDepth` cut.** Dropped from that mirror (§5.2). Intended: the projection
   is a view of one depth band.
10. **`minDepth` plus `collapsed`.** `collapsed` entries above the cut are moot — those levels
    generate no directories anyway. Entries below it behave normally.
11. **Foreign `include` with no matching pictures.** Every directory prunes to empty and the mirror
    renders nothing. Correct, and visible in the resolved-tree preview (§9) rather than a surprise on
    the phone.
12. **`trashedDuplicate: ignore` on a node whose client renames by delete + create.** The rename
    silently fails to restore the picture and the client sees a new file where the old one was. This
    is why the root seed is `restore` and only `drop` nodes default to `ignore`.
13. **A drop node's verification window expires before the client reads.** The retry re-`PUT`s,
    dedupes on `file_hash`, and re-opens the window (§7). Self-healing, at the cost of re-transferring
    the bytes.

## 13. Doc updates

All applied with the implementation (plus `05_FRONTEND_ARCHITECTURE.md` §7 for the editors).

- `01_GENERAL_SPECIFICATIONS.md §3.5` — **the invariant change**: `manual` tags are no longer
  unconditionally untouched. `incoming_share` still never; `manual` only by an opted-in service, at
  an identical path, down to the derived anchor.
- `01_GENERAL_SPECIFICATIONS.md §4` — one line: write-back is a transition between node chains.
- `05_hierarchies.md` — §4.3 (`mirror.include`, `minDepth`), §5.2 (`minDepth`, precise empty
  pruning), §7 (the difference rule and verb mapping replacing the per-verb description), §11
  (validation additions), §14 (new edge cases).
- `18_hierarchy_improvements.md` — §7.3 gains a pointer to §5.4 here (the field keeps both meanings
  and is not being consolidated); §5.1's resolution algorithm is cited by one more tri-state.
- `38_sensitive_pictures.md §2.1` — the stopgap caveat correction (§5.4), already applied.
- `06_webdav.md` — §7–8 (the transition model, `trashedDuplicate`), §9 (the drop-node verification
  listing, client compatibility, the Android recipe).
- `03_BACKEND_ARCHITECTURE.md §E` — the prune step at the end of a per-picture evaluation, and the
  recorded gate history it reads.
- `06_API_REFERENCE.md` — the hierarchy `config` shape (`version: 3`, three new fields); tagging
  service CRUD gains `prune_shadowed_manual_tags` and `keep_manual_tags`; disable moves from a
  `PATCH` field to a flow carrying `promote_tags`.
- `99_ROADMAP.md` — entry.

## 14. Work breakdown

1. [x] Migration `0023_prune_shadowed_manual_tags` — the two `tagging_services` columns (§4.1).
2. [x] Gate history — `domain::pipeline::prune::GateHistory`, fed by the existing per-picture /
   per-service loop (§4.3).
3. [x] The prune step — `GateHistory::plan`; applied in the `reconcile_pipeline_tags` statement
   (§4.4, §4.6, edge case 7).
4. [x] Service CRUD + validation for the two fields (§11); disable with `promote_tags` (§4.7).
5. [x] `domain::hierarchy` — `mirror.include`, `mirror.minDepth`, `trashedDuplicate`, `version: 3`.
6. [x] Resolver — `minDepth`, the foreign `include`, filtered-mirror path sets (§5.3).
7. [x] Node chains + the §3.1 difference (`resolver::node_chain` / `transition`), verb mapping in
   `services/vfs`.
8. [x] `trashedDuplicate` resolution and the split trashed-match branch; `drop` nodes created with
   `ignore` (frontend).
9. [x] Drop-node verification listing — Redis key, setting, listing/`GET`/`DELETE`/`MOVE`-out (§7).
   *Not done:* confirming the hypothesis from an access log (§8).
10. [x] Frontend — master-detail `HierarchyEditor` (`NodeTree` + `NodeSettings`), resolved-tree
    preview, the node controls, the §8.1 preset; `PruneSettings` and the `ServiceActions` disable /
    delete dialog.
11. [x] Tests — `domain::pipeline::prune` and `resolver` unit tests; `back/tests/pipeline.rs`
    (prune to an anchor and stable, no anchor, opt-in + backfill via `touch_invalidated`, keep-list,
    disable with promote); `back/tests/vfs.rs` (sibling move, move-out, PUT chain, `singleBranch`
    deepest-only, `fullDelete`, `trashedDuplicate` both ways, drop verification + zero window,
    `minDepth` writes, foreign `include`); `back/tests/hierarchy.rs` (projection, precise pruning,
    the two `exclude` meanings).

## 15. Deviations & choices taken while implementing

- **A satisfier from an upstream service survives the prune.** §4.4's "sole satisfier" is evaluated
  per gating path *with its origin*: a path an earlier service produced keeps satisfying a later gate
  after the manual row goes (that service keeps firing, since every fired gate is anchored), so it
  frees the arm. Candidates never count for each other, so two competing shadows both anchor.
- **Mirror chain nodes.** Each mirror *directory* is its own chain node (keyed by node id + tag), and
  only the deepest level of one expansion counts — so moving between two mirror directories fires
  their own ops, and moving out of `Era/2026/Vietnam/` removes `Era.2026.Vietnam`, not `Era`.
- **The 409 stays.** A transition still rejects removing a tag a service asserts (06_webdav §7.2).
- **Projection details.** Colliding projected leaves (case-insensitive) are all qualified by their
  suppressed levels (`Paris (2025)`); a `keepDir` container above a projection is read-only and has
  no direct files; a projected `keepDir: false` mirror is never a container's "first hoisted
  mirror", so a new top-level directory there is `403`.
- **Precise pruning by path set, not per-directory counts.** A filtered mirror expands from the paths
  its matching pictures carry (`PictureRepository::tag_paths_under`, one query per filtered mirror,
  ancestors' predicates included) plus `show_when_empty` tags. A filtered mirror also subtracts only
  its *matching* pictures from a parent query's direct files.
- **Drop inbox writes.** A same-named `PUT` into a drop node is never an overwrite (a new photo with
  a reused camera name); `ignore` returns `201`; the verification entry is best-effort (a Redis
  failure never fails the committed write).
- **Disable is a `PATCH` field.** `promote_tags` rides `PATCH {enabled: false}`; delete and disable
  share one checkbox dialog in the UI. The editor's "by gates" list is the enabled services'
  `requires` arms related to this service's outputs — an approximation of the per-picture anchors.
