# Tag metadata

## 1. Overview & goals

Tags today have **no storage of their own**: `tags` is `(picture_id, tag_path, source)` and the whole
tree is reconstructed from distinct paths. A tag is a pure derived string, which means it cannot carry
a human name, a date, a cover, an order, or exist before it has pictures.

This feature adds one **purely decorative** side table, `tag_metadata`, keyed by `(user_id, tag_path)`.
Nothing in the engine reads it: the pipeline, hierarchies, sharing and federation continue to see only
`tags`. If a row is absent, behaviour is exactly what it is today.

It is the substrate for [35_timeline_view.md](35_timeline_view.md) and it closes four long-standing
gaps:

- **Names** — `12_Christmass_Alps` can display as "Christmas in the Alps 🎄" without widening the
  ltree alphabet or touching the WebDAV/share/rule surfaces that consume paths.
- **Dates** — a tag gets a date range, so ordering stops living in the slug.
- **Empty tags** — a tag can exist with no pictures, which is what lets an event be created before it
  is filled, and what removes the WebDAV `MKCOL` placeholder hack (§8).
- **Order** — per-parent ordering including manual drag order.

## 2. Decisions (settled)

- **Metadata is decorative, never engine input.** No pipeline gate, hierarchy resolver or federation
  path may read `tag_metadata`. The moment it becomes engine input, tags stop being the single
  primitive and every subsystem grows a second dependency. There is **one** deliberate exception,
  WebDAV (§8): directory *naming*, and the *existence* of an empty directory. Both are presentation
  of the path set, not membership — no picture's tags change either way — but the second one does mean
  the VFS enumerates from `tag_metadata`, so it is named here rather than left implied.
- **The ltree path stays the identity.** Display names are additive. Cosmetic renames become one
  `UPDATE`; slug renames keep the existing async cascade (§12).
- **Reserved prefixes are allowed here**, unlike manual tag assignment. A user may name
  `SharedToMe.alice_AT_instance_DOT_com` "Alice" — this generalises the `decodeLabel` helpers
  duplicated across `TagTree.tsx`, `SelectionPanel.tsx` and `lib/utils.ts` (§5).
- **Rows are never auto-deleted.** Untagging every picture leaves the row intact; only an explicit
  *Reset metadata* removes it. Losing a display name and cover because a tag momentarily emptied is
  the worse failure, and the volume is bounded by tags-ever-created (~200 B each).
- **`show_when_empty` is visibility, not retention.** Independent of the above: it controls whether a
  zero-picture tag appears in the tree. Default `true` for a deliberately-created empty tag, `false`
  when merely annotating an existing one.
- **No row for an all-default state.** An upsert whose every field equals its default (and
  `show_when_empty = false`) deletes the row instead. Browsing with default view settings must not
  litter the table.
- **Derived dates are computed, not stored.** Only the *override* is persisted (§4).
- **No auto-derived cover.** Resolving a cover when `cover_picture_id` is NULL would need an
  `array_agg` over the ancestor-expanded lateral; the frontend falls back to the first photo it has
  loaded, at zero cost. Revisit if it looks bad.
- **No icon field.** `display_name` already accepts emoji, so a separate glyph column would only ever
  render beside the name it decorates — while needing its own picker, validation and fallback. An
  emoji typed into the name travels everywhere the name travels. `color` stays, because a tint on a
  tree row or a block accent is the one decoration that cannot live inside the text.
- **The server never sorts or orders tags.** It returns the set and its metadata; the client resolves
  ordering (§7). This is why there is no reorder endpoint — a drag is an ordinary `sort_index` write.
- **The root is a tag row.** The no-tag gallery view is `tag_path = ''` (§3.3), not a set of
  `user_settings` columns, so it gets view mode, grouping, child ordering and subtag placement on the
  same code path as every other node.

### 2.1 Rejected: an `intervals` table

An earlier design made eras/trips/events **date intervals** that generate tags by `captured_at`
containment, with membership automatic and nesting derived from date containment rather than asserted
by path. It was rejected on field experience, and the reasoning is worth keeping:

- Selecting pictures and assigning a tag is **faster in practice** than declaring an interval.
- Intervals break on pictures with no `captured_at`, which are common.
- A received share routinely contains pictures from just before/after the event; two-click share
  mapping handles that correctly where a date rule cannot.

Events and trips are therefore **ordinary manual tags**. The only thing they needed was metadata —
which is this feature. An optional "also create a date rule" checkbox at creation remains possible
later, but its value is narrow (own photos imported late) and it is not part of this work.

## 3. Schema

```sql
CREATE TYPE tag_order            AS ENUM ('manual','date_from','date_to','path','display_name');
CREATE TYPE tag_view_mode        AS ENUM ('direct','subtag','all');
CREATE TYPE tag_subtag_placement AS ENUM ('top','in_sections');

CREATE TABLE public.tag_metadata (
    user_id          uuid          NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tag_path         ltree         NOT NULL,              -- '' = the root view (§3.3)
    display_name     varchar(128),
    description      varchar(2000),
    cover_picture_id uuid          REFERENCES pictures(id) ON DELETE SET NULL,
    color            varchar(16),                         -- '#RRGGBB' (§3.2)
    date_from        timestamp,                           -- NULL ⇒ derived (§4)
    date_to          timestamp,
    show_when_empty  boolean       NOT NULL DEFAULT false,
    sort_index       integer,                             -- slot among siblings (§7)
    children_order   tag_order     NOT NULL DEFAULT 'manual',    -- how MY children sort
    view_mode        tag_view_mode NOT NULL DEFAULT 'subtag',    -- feature 35
    subtag_placement tag_subtag_placement,                -- NULL ⇒ derived (§3.4)
    grouping         jsonb         NOT NULL DEFAULT '{}'::jsonb, -- §3.1, feature 35 §4
    webdav_dir_name  varchar(255),                        -- §8
    created_at       timestamp     NOT NULL DEFAULT (now() AT TIME ZONE 'utc'),
    updated_at       timestamp     NOT NULL DEFAULT (now() AT TIME ZONE 'utc'),
    PRIMARY KEY (user_id, tag_path)
);
CREATE INDEX idx_tag_metadata_path  ON public.tag_metadata USING gist (tag_path);
CREATE INDEX idx_tag_metadata_cover ON public.tag_metadata USING btree (cover_picture_id)
    WHERE cover_picture_id IS NOT NULL;
```

`user_id` is explicit because `tags` has no user column — ownership flows through
`pictures.local_user_id`, which a metadata row cannot inherit. The PK btree already narrows to one
user, so the GiST index only has to narrow the prefix for the rename swap (§12); a composite
`gist (user_id, tag_path)` would need `btree_gist` and buys nothing here. `idx_tag_metadata_cover`
exists so a bulk purge does not scan the table once per deleted picture to satisfy
`ON DELETE SET NULL`.

Season grouping needs a hemisphere: add `user_settings.hemisphere` (`north` | `south`, default
`north`) rather than reaching into a segmentation service's config. It is the **viewer's** convention,
applied to every photo regardless of where it was taken.

### 3.1 `grouping` shape

Grouping buckets the **sort field**, because sections must be contiguous runs in the sorted order —
group by month while sorted by filename and a "month" would reappear a dozen times. So the stored
value is a map **keyed by sort field**, which also means switching sort back and forth does not lose
the setting:

```jsonc
{
  "captured_at": { "kind": "month" },              // none|year|quarter|season|month
  "ingested_at": { "kind": "year" },               // also updated_at
  "filename":    { "kind": "prefix", "chars": 3 }, // IMG… / DSC… / PXL…
  "file_size":   { "kind": "magnitude" },          // byte decades
  "geo_near":    { "kind": "magnitude" },          // metric distance decades, over distance_m
  "time_near":   { "kind": "magnitude" }           // time ladder, over |captured_at − near_time|
}
```

A missing key means that sort field's default: `month` for the three date fields, `none` for
everything else. `magnitude` is a single `kind` carrying a **per-field ladder** (feature 35 §4) rather
than three separate kinds, so a new bucketable field ships by declaring its ladder.

JSONB rather than an enum + parameter column follows the house precedent (`hierarchies.config`,
`tagging_services.config`, feature 13 predicates) and lets a new `kind` ship without a migration.

### 3.2 Validation

- `tag_path` is a valid ltree path; **reserved prefixes permitted**, and the empty path is permitted
  **only** as the root sentinel (§3.3). `TagPath::parse` rejects the empty string, so no real tag can
  ever collide with it; the metadata layer carves out the one exception rather than relaxing `parse`.
- `display_name` trimmed, 1–128 chars, Unicode and emoji allowed, control characters rejected.
  NULL means "not overridden" — an empty string is rejected, not stored.
- `description` trimmed, ≤2000 chars, control characters other than newline rejected.
- `color` matches `^#[0-9a-fA-F]{6}$`. Stored as free hex so a custom picker is possible; the UI
  offers a fixed palette by default and the picker behind a "custom" affordance.
- `date_from <= date_to` when both are set. Each side is independently overridable.
- `cover_picture_id` must belong to the caller.
- `webdav_dir_name` contains no `/`, no control characters, is trimmed and non-empty, ≤255 chars.
- `grouping` keys are known sort fields; each `kind` is valid **for that field**; `prefix.chars` is
  1–16. Unknown keys are rejected rather than ignored, so a typo surfaces instead of silently
  reverting to the default.
- Sibling `display_name` collisions are **not** rejected — a soft UI warning only.

### 3.3 The root row

`tag_path = ''` (the zero-level ltree) is the gallery's no-tag view. It is an ordinary row: it carries
`view_mode`, `grouping`, `children_order` and `subtag_placement`, and it may carry a `display_name`
and `cover` too (harmless; only the view fields are read). This is why there is no
`user_settings.root_children_order` — root ordering is `children_order` on the root row, reached by
exactly the same code as any other parent.

Two consequences:

- **`direct` at the root means untagged.** `TagPredicate.untagged` already exists and is already wired
  through `build_flat_predicate`, so the three modes stay uniform: `direct` = mine alone, `subtag` =
  mine + child blocks, `all` = everything. Untagged pictures gain a home they do not have today.
- **The root row is never renamed** and is skipped by the §12 prefix swap, since `'' @> anything` is
  true and a swap rooted at `''` would rewrite the whole table.

Implementation check: this relies on `''::ltree` being a valid zero-level path (`nlevel('') = 0`).
Confirm with one `SELECT nlevel(''::ltree);` before writing the migration.

### 3.4 `subtag_placement`

Where a parent's child blocks sit relative to its grouping sections (feature 35 §5). `top` puts them
once, before the first section; `in_sections` places each child in the section its date falls into.

`in_sections` is only **meaningful** when the active sort field has a per-tag range to place against —
i.e. `captured_at`, `ingested_at`, `updated_at`. A tag has no filename, size or distance, so under any
other sort the resolver falls back to `top` whatever is stored. This replaces what was previously a
hidden special case in the rendering rules with one stored, user-visible option.

NULL means derived, and the derived default is deliberately not uniform:

| Tag | Date sort | Non-date sort |
|---|---|---|
| Root (`''`) | `top` | `top` |
| Any other tag | `in_sections` | `top` |

The root's children are namespaces (`Era`, `Clients`, `Screenshots`) whose date ranges span
everything, so scattering them through months would be noise; a nested tag's children are events,
where chronological placement is the point. Grouping itself still defaults to `month` in both cases.

## 4. Derived dates and the cache

`date_from`/`date_to` default to `MIN/MAX(captured_at)` over the pictures carrying the tag,
**ancestor-expanded** so a parent's range covers its whole subtree. An explicit `date_from`/`date_to`
replaces the derived value on that side only, in every trash mode.

This needs a **new whole-library query** — `TagRepository::list_tags_enriched(user_id)`. It is *shaped*
like `aggregate_tags` (the same `generate_series`/`subpath` prefix lateral) but it is not that
function: `aggregate_tags` takes a `ResolvedSelection` and serves the feature-14 batch panel, while
`GET /tags` today calls `list_paths_by_user`, which returns distinct paths and no counts at all. The
new query emits, per ancestor-expanded prefix:

| Field | SQL |
|---|---|
| `count` | `COUNT(DISTINCT picture_id) FILTER (WHERE deleted_at IS NULL)` |
| `exact_count` | as above, `AND pfx.prefix = tg.tag_path` |
| `trashed.count` / `trashed.exact_count` | the same two, `FILTER (WHERE deleted_at IS NOT NULL)` |
| `date_from` / `date_to` | `MIN/MAX(captured_at) FILTER (WHERE deleted_at IS NULL)` |
| `trashed.date_from` / `trashed.date_to` | the same, `FILTER (WHERE deleted_at IS NOT NULL)` |

One pass, six extra aggregates, no second query. The trashed half is what lets the trash view keep its
structure instead of losing every tag whose pictures are all deleted (feature 35 §10); the client
picks the pair matching the active trash filter, and the *All* mode takes the min/max of both. A tag
with `count = 0` and `trashed.count > 0` is **hidden in the default view** and appears only under
trash `include` or `only` — otherwise trashing an event would leave a permanent empty node behind.

No index makes this cheap: it is a full scan of the user's `tags` joined to `pictures`, expanded by
`nlevel`. `idx_pictures_captured` is a plain btree on the column and does not help a per-user
group-by. The cache below is what makes the cost affordable, not an index.

**Cache.** Redis `tags:tree:{user_id}`, one JSON blob for the whole tree (without `sources`), **60 s
TTL** plus explicit bust. A row-level trigger on `tags` was rejected: it would fire per row during
bulk pipeline runs. A TTL is defensible because the tag tree is *already* eventually consistent — the
pipeline is async, so users already experience convergence rather than immediacy.

**Bust points.** Not the pipeline *wake* set, which is the trap: `POST /uploads/{id}/complete` wakes
the pipeline and returns, so busting there repopulates the cache from pre-pipeline state and then
holds it for 60 s — precisely the window where the user is watching for their new tags. Bust at the
**end of a pipeline run** instead. Plus the synchronous writers: `PATCH /tags`, `POST /tags/rename`,
tagging-service CRUD, picture trash/restore/purge, EXIF writes touching `captured_at`, share
accept/revoke, and any `tag_metadata` write. WebDAV `PUT`/`MKCOL` mint tags outside the API and are
left to the TTL.

**Client.** The enriched tag list is fetched once at app start, refetched on a **5-minute** interval,
and invalidated immediately on local mutation. Payload is ~120 B/tag, so a thousand-tag library is
well under 200 KB.

This is the whole read path for *browsing*. Everything the frontend needs to render, name, order, date
and group the tag tree is in that one payload, so expanding, reordering, switching view mode or
grouping, and navigating between tags issue **no further tag queries**. The two exceptions are writes
(§4.1) and the edit dialog, which fetches `GET /tags?with_sources=true` on open for the provenance
breakdown — the path×source query is too heavy to carry in the app-start payload.

### 4.1 Write debouncing and flush

View preferences and ordering change far more often than they need persisting — a user nudging the
grouping five times in half a minute must not produce five round trips. All `tag_metadata` writes go
through one queue, keyed by `tag_path`, that **coalesces per tag** (last value wins per field) and
flushes on a **60 s trailing debounce**. A flush sends the whole coalesced batch as one
`PUT /tags/meta` (§11 takes an array), so touching ten tags is still one request.

Immediate flush regardless of the timer on:

- navigating away from the tag whose preferences changed,
- `visibilitychange` → `hidden` (covers tab switch and mobile backgrounding, which on iOS is often the
  last event before the page is discarded),
- `pagehide` / `beforeunload`.

The unload flush uses **`fetch(..., { keepalive: true })`, not `navigator.sendBeacon`**: auth is a
`Bearer` header set by the axios interceptor in `api/client.ts`, and `sendBeacon` cannot set headers.
`keepalive` allows them and survives document teardown, within a 64 KB body cap that a coalesced
preferences payload is nowhere near.

A raw `fetch` also escapes the response interceptor's 401 → refresh → retry, and at `pagehide` there
is no document left to run a refresh in. The queue therefore **refreshes proactively**: if the access
token is within its last minute of validity when a flush is triggered, it refreshes first and only
then sends. An expired-token flush is otherwise silently lost.

Optimistic locally, so the UI never waits on the debounce; a failed flush re-queues once and then
surfaces a toast rather than silently discarding. Because a flush can arrive after the 5-minute
re-fetch, writes carry only the fields that actually changed (`PUT /tags/meta` is a partial upsert),
so a stale full-row write can never clobber a concurrent change from another device.

## 5. Display-name resolution

The rule: **the ltree path is never hidden on a surface that writes it.**

Display names do not replace the `decodeLabel` helpers (`_AT_` → `@`, `_DOT_` → `.`) — those remain the
fallback for every tag with no metadata row. Today they are copy-pasted into `TagTree.tsx:27`,
`SelectionPanel.tsx:64` and `lib/utils.ts:96`; resolution becomes one helper
(`display(path) = meta.display_name ?? decodeLabel(leaf(path))`) and the copies go.

The table is exhaustive on purpose — §5 is a policy, and a policy is only enforceable at review time
if the surfaces are enumerated:

| Surface | Shows |
|---|---|
| Tag tree | display name; full path on hover and in the row's `…` menu header; the ltree *leaf label* shows beside the name **only** on a sibling-name collision |
| Tag picker / autocomplete | the **whole** path with display names substituted (`displayPath`), with the raw ltree path muted on a second line when it differs; **search matches both**. A picker writes the path, so a display name renames a segment here — it never hides where the tag lives |
| Filter chips (`TagFilterBar`), breadcrumbs, group headers | display name; path in tooltip |
| Timeline subtag blocks (feature 35) | display name; path in tooltip |
| `SelectionPanel` tag chips (single selection) | the **whole** path with each segment's display name substituted (`displayPath`); raw ltree path in tooltip — a flat chip list has no tree to place a bare leaf in |
| `MultiSelectionPanel` tristate tags (feature 14) | display name; path in tooltip |
| "Shared with you" / "Shared by you" sections | display name of the shared subpath, sender handle raw |
| Lightbox tag overlay | display name |
| Hierarchy directory tree (webapp browse) | directory name as the hierarchy resolves it — *not* display name |
| Segmentation template editor | **raw paths only**, plus a preview line (`Era.2024.August` → *August 2024*) |
| Rule `assign_tag`, hierarchy config, share target | raw path, display name muted alongside |
| WebDAV | `webdav_dir_name` → ltree label (§8). Never `display_name`. |

**Creation.** Typing `Vietnam 🇻🇳 2024` into the picker's create field runs the existing
`TagPath::slugify_label` to mint `Vietnam_2024` and sets `display_name` to what was typed. A sibling
slug collision is **rejected** with "that tag already exists" rather than auto-disambiguated — a
silent `_2` suffix is worse than asking.

**Sibling name collisions.** Allowed and warned (§13.6), but two identically-named siblings must not be
indistinguishable in a picker: when a collision exists, the ltree label is forced visible (not muted)
on every colliding entry.

## 6. Edit tag dialog

One dialog from the tag tree's `…` menu, absorbing today's `RenameTagDialog`. Fields map 1:1 to §3:

- **Display name** — placeholder shows the ltree label; emoji encouraged here, there is no icon field
- **Path** — the rename control, warning that the cascade runs asynchronously (§12)
- **Description**
- **Cover** — picker over pictures carrying this tag, plus Clear
- **Date range** — two fields, placeholders showing the derived values, per-side *reset to derived*
- **Color** — fixed palette, plus a custom hex picker behind "Custom"
- **Show when empty**
- **WebDAV folder name** — with a *use display name* shortcut that fills it once, explicitly (§8)
- Footer: **Reset metadata** — deletes the row, keeps the tag

Read-only header: total count, direct (exact) count, trashed count when non-zero, source breakdown
(fetched on open via `?with_sources=true`), and a one-line *"Shared with 3 people →"* linking out.

**Reset metadata is worded by consequence, not by mechanism.** For a tag that still has pictures it
reads "Reset metadata — the tag and its photos stay". For an empty `show_when_empty` tag the row *is*
the tag, so it reads "Delete tag" and confirms as a deletion. Either way the confirm names what
disappears (display name, description, cover, dates), because there is no undo.

**Creating an empty tag.** §1 promises an event can exist before it is filled, so the tag tree needs a
**New tag** action (root level and under any node) that mints the row with `show_when_empty = true` and
no pictures. Without it the only path to an empty tag is WebDAV `MKCOL`, which is not a path most users
have.

**Share management stays out of this dialog.** Editing a label and granting access are different
intents; mixing them makes the modal a place people revoke things by accident.

## 7. Ordering

`children_order` on a tag decides how **its children** sort; `sort_index` is a child's own slot. Root
ordering is `children_order` on the root row (§3.3).

**Ordering is resolved entirely in the frontend.** `children_order`, `sort_index`, `display_name` and
the derived dates all ship in the one app-start payload (§4), so the client has everything it needs to
sort any level of the tree — there is **no ordering query, and no per-view or per-tag query of any
kind**. The server never sorts tags; it returns the set and its metadata.

Client-side, manual order is `sort_index` ascending with unset last, tie-broken by
`display_name ?? label`, so **manual and alphabetical coincide until the first drag** — reordering
needs no mode switch and an unconfigured tree is alphabetical, as expected.

**Entering reorder mode numbers the list.** A sibling with no stored `sort_index` sorts *last*, so in
an unconfigured list the first single-row write makes that row jump to the front instead of moving —
a move down was simply impossible. `initialOrderWrites` materialises the resolved order as explicit
indices on entry (one coalesced batch), and `reorderWrites` falls back to a full renumber whenever any
sibling is still unnumbered.

**A drag is an ordinary metadata write.** There is no reorder endpoint. The client knows the resolved
order, so it computes the moved node's new `sort_index` as the midpoint of its new neighbours'
effective indices and sends **one** `PUT /tags/meta { tag_path, sort_index }` through the §4.1 queue.
Effective index for a sibling with no stored `sort_index` is its position in the current resolved order
× 100, which is what makes the first drag in an unconfigured list a single-row write instead of
materialising a row per sibling. Only when two neighbours leave no integer between them does the client
renumber that one sibling list in sparse steps of 100 and send the batch — rare, and still one request.

**`children_order` follows the drag.** Dragging under a parent sorted by anything but `manual` is
meaningless, so entering reorder mode on such a parent switches it to `manual` and says so inline
("Subtags now sort manually").

**Reorder UX.** Rather than making the tree draggable inside a scrolling panel, *Reorder subtags* in
the `…` menu switches that subtree into an explicit reorder mode — drag handles on desktop, up/down
buttons on touch. Avoids accidental drags and is discoverable on mobile, where free-form drag in a
scroll container is unreliable.

## 8. WebDAV directory names

`webdav_dir_name` resolves as: **explicit value → ltree label**. It is *never* derived from
`display_name`.

The reason is sync stability: a mounted client sees a directory rename as delete + create, so
deriving the folder name from a display name would churn every synced client whenever someone
tidied a label. It is set only by `MKCOL` (§8.1) or deliberately in the edit dialog.

**Resolution order is load-bearing.** 06_webdav.md §9 slugifies an unrecognised segment under a mirror
into a *new* tag, so a near-miss on a custom folder name would silently mint `Vietnam_2024_` instead of
resolving. Path→tag resolution is therefore strictly: **custom `webdav_dir_name` → ltree label →
slugify-and-mint**, served from the per-user tag payload already cached in memory.

Collisions within one directory fall back to the ltree label for the later sibling, deterministic by
path sort; an authored `static`/`query` node name always wins over a custom mirror name. Case
folding (06_webdav.md §10c, `VirtualFs::fold_case`) applies to custom names too: a `webdav_dir_name`
that differs only by case from a sibling's effective name is treated as a collision and falls back the
same way, so a case-insensitive client never sees two directories it cannot tell apart.

`MOVE` on a collection is out of scope (99_ROADMAP "Advanced WebDAV") with **one** exception: a
still-empty `show_when_empty` directory, renamed in place under the same parent. That is Finder's
create-then-rename flow — it mints `untitled folder` with `MKCOL` and immediately `MOVE`s it to the
typed name — and without it every folder created in Finder keeps its placeholder name.

The rename is a `tag_metadata` prefix swap (§12), *not* a tag rename, so it is only taken when the
old subtree carries no picture **at all, trashed included**: a trashed picture still holds the old
path, and re-filing those belongs to the `tag_rename` cascade. When one does, the destination gets a
fresh empty tag and the source keeps its row with `show_when_empty` cleared — the decoration
survives for when the pictures come back (§13.9). Reparenting and a non-empty directory stay `405`.

### 8.1 `MKCOL` on a mirror node

`show_when_empty` **is** the fix for directory creation. `MKCOL "Vietnam 2024 🇻🇳"` under a mirror
node now:

1. slugifies to a valid label (`Vietnam_2024`),
2. inserts `tag_metadata` with `display_name` and `webdav_dir_name` set to the requested name and
   `show_when_empty = true`,
3. returns 201 — and the directory **persists and lists** with no pictures in it.

This removes the `webdav:pendingdir:*` Redis sidecar and the `WebdavPendingDir` key entirely. The
conflict check at the top of `VirtualFs::mkcol` must now consider custom names as well as labels. The
**other** sidecars are untouched: the dotfile / OS-junk store (06_webdav.md §11) and the Preview
temp-file behaviour (features/08) stay exactly as they are.

Deleting the directory deletes the metadata row — but only when `show_when_empty` is what made the
directory exist and the tag carries no picture, live or trashed. A directory can also be empty
because a foreign `exclude` (18 §7.3) cut every one of its pictures, and wiping a live tag's
decoration for that would be silent, undoable loss. Nothing is untagged either way: a DELETE on a
directory whose contents the client cannot see must not mutate them.
An empty tag appears in *every* mirror hierarchy whose prefix covers it, which is the same rule a
non-empty tag already follows.

## 9. Drag-and-drop tagging

Dragging photos onto a tag assigns it. Backend cost is zero — it is
`PATCH /tags` (`batchEditTags`) with the existing selection descriptor.

- **Source** — a `PhotoCard`. If the dragged card is in the current selection, drag the whole
  selection; otherwise select it and drag it alone. The ghost carries a count badge.
- **Targets** — tag tree nodes, and collapsed subtag blocks in the timeline (feature 35), identically.
- **Rejected** — `SharedToMe.*` nodes (reserved prefix), shown with a not-allowed cursor.

| | Non-sibling target | Sibling target |
|---|---|---|
| **Single photo** | assign silently | dialog: also remove the source tag? |
| **Batch** | confirm dialog | primary *"Add and remove Vietnam"* · secondary *"Add only"* |

"Sibling" is defined only when the drag starts inside a subtag block in Subtag view; from the flat
grid there is no source tag, so it is always the left column. The batch dialog shows real figures from
`batchEditTags({dry_run: true})` rather than a raw count.

Every drop toasts with **Undo**. The undo is scoped to the dry run's `added` set — the pictures that
actually gained the tag — not to the whole selection, which would strip the tag from pictures that
already carried it.

**Desktop only.** HTML5 `draggable` does not fire on touch, so it cannot collide with the
`onLongPress` multi-select in `PhotoGrid` — and the batch panel already covers mobile.

## 10. Shares in the tag tree

Steady-state share *browsing* moves onto the tag tree; the Shares tabs stay for what is
**actionable**.

**On a node:** an avatar stack (≤3) or `Share2` + count for outgoing, `Link2` for public links, a
"from @alice" marker on `SharedToMe` sender levels and on locally-mapped tags. Descendants of a shared
tag get a **fainter inherited marker** — without it nobody realises that sharing `Era.2024` exposed
`Era.2024.Vietnam`. All of it is computed client-side by prefix from the share lists already fetched.

**Badge click → popover** (the `…` menu keeps structural actions): recipients with status, `future`
and `allow_exif_edit` flags, revoke, "Share with someone else…", public links with copy/revoke, and —
for a share-mapping target — the sender and an unmap action. Detecting that last one is a JSONB
containment query over `tagging_services` (`config->>'incoming_share_id'` set and `config->'assignTags'`
containing the path); a handful of rows per user.

**Tabs:** a count badge on the nav item for `pending` incoming shares. Incoming grouped by sender,
outgoing grouped as a tag tree.

### 10.1 Metadata across the share boundary

A recipient currently sees `SharedToMe.alice_AT_instance_DOT_com.Vietnam_2024` while the sender has
"Vietnam 🇻🇳" sitting right there. The sender's decoration travels.

**What travels:** `display_name`, `description`, `color`, `cover_picture_id`. Not `webdav_dir_name`
(the recipient's mount is theirs), not ordering, not view preferences.

**When:** on the **share announcement** (feature 02), not the per-picture announcement — the metadata
describes the share's tag, not each photo, and repeating it per picture would be noise.

**Seed once, never overwrite.** The recipient's `tag_metadata` row for
`SharedToMe.<sender>.<shared subpath>` is created from the announced values **on first accept only**.
The share state machine re-announces on update (feature 02), so without this rule a sender tidying
their own label would silently clobber a recipient who had renamed it. A recipient who resets their
metadata gets the tag back at its raw path, not re-seeded.

**Only the `SharedToMe` node.** A share-mapping target is a tag the recipient already owns and may
already have named; it is never seeded.

**Cover resolution.** The cover travels as the owner's picture id. The recipient resolves it through
`pictures.remote_picture_id`; if the picture is not (or not yet) in the share, the cover is dropped and
the frontend's first-loaded-photo fallback applies.

**Against the share's own `name` / `message`.** Both `outgoing_shares` and `public_shares` already
carry `name varchar(64) NOT NULL` + `message text`, and they stay. They are about *this act of
sharing* — addressed to one recipient, "Vietnam pics, grab what you want" — while display name and
description are about the subject and identical for everyone. The create-share dialog **prefills
`name` from the tag's `display_name`** (still editable, so a share can be labelled differently) and
leaves `message` empty. On the receiving side the share's `message` renders as the sender's note in
the share UI, never as the tag's description.

**Public shares (feature 27)** read the same fields with no protocol involved: the public landing page
renders the tag's display name, description, cover and colour instead of a bare slug. This is a purely
local read and ships with this feature.

## 11. API

```
GET    /api/authenticated/tags             enriched (below); `with_sources=true` adds provenance
PUT    /api/authenticated/tags/meta        { items: [ { tag_path, ...fields } ] }   partial upsert
DELETE /api/authenticated/tags/meta        { tag_paths: [ ... ] }
POST   /api/authenticated/tags/rename      (existing, extended — §12)
```

ltree paths carry dots, so they travel in the body rather than a URL segment. `PUT` and `DELETE` take
arrays because the §4.1 queue flushes a coalesced batch; a single-item array is the common case.
There is **no reorder endpoint** — `sort_index` is an ordinary field on the upsert (§7).

`with_sources` currently only means anything alongside `picture_id`; it now applies to the
whole-library branch too, where it triggers the heavier path×source query. The app-start payload never
sets it.

`GET /tags` changes shape from `{ tags: string[] }`:

```jsonc
{ "tags": [ { "path": "Era.2026.Vietnam", "count": 184, "exact_count": 122,
              "date_from": "2026-08-01T09:12:00", "date_to": "2026-08-14T21:40:00",
              "trashed": { "count": 3, "exact_count": 3,          // omitted when zero
                           "date_from": "...", "date_to": "..." },
              "sources": [...],            // only with with_sources=true
              "meta": { "display_name": "Vietnam 🇻🇳", ... } | null } ] }
```

The root row is returned as `"path": ""` with its `meta` and no counts. Tags with no live pictures
appear iff they have a metadata row with `show_when_empty = true`, or they have trashed pictures (§4,
surfaced only under trash `include`/`only`).

The shape change has exactly two consumers today — `TagPicker.tsx:93` and `TagTree.tsx:226`.

## 12. Rename cascade

`TagMetadataRepository::rename_subtree(user_id, old, new)` — an ltree prefix swap shaped exactly like
`TagRepository::rename_manual_subtree` — joins the existing `tag_rename` routine, followed by a cache
bust. On PK collision the **target row wins and the source is dropped**, mirroring how colliding
manual tag rows are already handled.

The root row (`tag_path = ''`) is excluded from the swap: `'' @> anything` is true, so a swap rooted
there would rewrite every row the user has.

`POST /tags/rename` rejects reserved paths, so a `SharedToMe.*` metadata row can never be moved by the
cascade. That is intended — those paths are structural — but it means a recipient's renamed share tag
survives only as long as the sender keeps the share at that path.

`webdav_dir_name` is *not* rewritten by a rename: it is an explicit override, and a path change does
not imply the user wants their mounted folder renamed.

## 13. Edge cases

1. **Resurrection.** Delete `Album.Test`, later recreate it, and the old display name returns because
   rows persist. The broken-cover half is already handled (`ON DELETE SET NULL`). Accepted — it is
   usually what the user wants — with a one-time inline hint on the tag ("restored name from a
   previous tag · reset").
2. **Metadata on a pipeline-generated tag.** Annotating a `rule`/`segment` tag leaves
   `show_when_empty = false`, so if the service stops producing it the tag simply disappears and no
   ghost remains in the tree. The row survives for when it returns.
3. **Cover points at a received picture whose share is revoked.** The picture row is deleted →
   `ON DELETE SET NULL` → the tag falls back to the frontend's first-loaded-photo cover.
4. **Empty tag under a mirror hierarchy node.** It renders as an empty directory. Intended — that is
   the `MKCOL` result (§8.1).
5. **Date override on a tag with no pictures.** Legal and useful: an event created ahead of time sorts
   into the right place in the timeline before it has a single photo.
6. **Sibling display-name collision.** Allowed, warned, and the ltree label is forced visible on the
   colliding entries (§5). Two "Vietnam" tags under different parents are legitimate; under the same
   parent it is the user's call.
7. **`display_name` containing `/`.** Allowed — it is never parsed. It is *not* propagated to
   `webdav_dir_name`, which rejects `/` (§3.2).
8. **The unload flush does not complete.** `keepalive` is best-effort: a hard kill or an offline device
   can still drop it, and the proactive refresh (§4.1) covers only an expiring token, not a dead
   network. Preference loss is cosmetic and self-healing — the next change re-queues — so there is no
   retry journal. A reorder is the one write where loss is visible, which is why `sort_index` goes
   through the same queue but is flushed on navigation away from the tree.
9. **A tag whose pictures are all trashed.** It keeps its metadata row, disappears from the default
   tree, and reappears under trash `include`/`only` with its trashed counts and range (§4). Restoring
   any picture brings it back with its name intact.
10. **The root row and the prune-if-all-default rule.** Setting the root's grouping and then resetting
    it deletes the root row like any other. Nothing depends on the row existing.

## 14. Doc updates

- [x] `01_GENERAL_SPECIFICATIONS.md §1` — tags may carry optional metadata, decorative only.
- [x] `01_GENERAL_SPECIFICATIONS.md §6` — tag metadata seeded on share accept (§10.1).
- [x] `06_API_REFERENCE.md §6.7` — the reshaped `GET /tags`, plus `PUT`/`DELETE /tags/meta`; §6.1
  gains `hemisphere`; §5 gains `tag_meta` on the public-share meta payload.
- [x] `05_FRONTEND_ARCHITECTURE.md §7` — edit dialog, new-tag action, share badges/popover,
  drag-and-drop, reorder mode; §5 the `tagDrag` store; §9 the write queue and drop/undo rules.
- [x] `features/02_pipeline_announcement_robustness.md §3` — the announce payload carries tag metadata.
- [x] `features/06_webdav.md §9` — `MKCOL` mints a `show_when_empty` row instead of the pending-dir
  sidecar; custom directory names and their resolution order. `§10c` — case folding applies to them.
- [x] `features/27_public_shares.md §15` — the public landing page renders tag metadata.
- [x] `99_ROADMAP.md` — entry moved to Done; collection `MOVE` noted under Advanced WebDAV.

## 15. Work breakdown

1. [x] Migration `0018_tag_metadata` — three enums, the table, two indexes,
   `user_settings.hemisphere`, and `incoming_shares.sender_tag_meta` (§10.1 parks the announced
   decoration until accept). `nlevel(''::ltree) = 0` confirmed.
2. [x] `TagMetadataRepository` — batch partial upsert (with the prune-if-all-default rule), batch
   delete, `rename_subtree`, list-for-user; `grouping` JSONB and `color` validation per §3.1/§3.2.
3. [x] `TagRepository::list_tags_enriched` — counts, exact counts, trashed counts and both date
   ranges in one pass (§4); Redis `tags:tree:{user_id}` with TTL, end-of-pipeline-run bust and the
   §4 bust set.
4. [x] Reshape `GET /tags`; add `PUT`/`DELETE /tags/meta`; extend the `tag_rename` routine with the
   metadata prefix swap and the root-row exclusion.
5. [x] WebDAV: `webdav_dir_name` in path resolution (custom → label → slugify) and `PROPFIND`
   listing; `MKCOL` creates the row; `WebdavPendingDir` and its sidecar removed.
6. [x] Frontend `useTags` for the enriched payload (5-min refetch); client-side ordering resolution
   (§7); one `display()` helper replacing the three `decodeLabel` copies (§5).
7. [x] The §4.1 write queue — per-tag coalescing, 60 s trailing debounce, batch flush,
   `visibilitychange`/`pagehide` flush via `fetch(keepalive)` with proactive token refresh,
   optimistic local state.
8. [x] `EditTagDialog` absorbing `RenameTagDialog`; the **New tag** action; consequence-worded reset.
9. [x] Tag tree: share badges + popover, reorder mode, empty-tag rendering, colour.
10. [x] Drag-and-drop with the §9 dialogs and the undo (offered only when every selected picture
    gained the tag — see §16).
11. [x] Share metadata propagation (§10.1) — announce payload, seed-on-first-accept, cover resolution
    via `remote_picture_id`, `name` prefill in the create-share dialog, public landing page.
12. [x] Tests: prune-if-default, rename collision, root-row exclusion from the swap, reserved-prefix
    metadata, `MKCOL` round-trip, custom-name resolution precedence, derived-vs-override dates,
    trashed counts and visibility, seed-once-on-accept. Write-queue coalescing is covered by the
    build only — the frontend has no test runner.

## 16. Deviations taken while implementing

- **Undo after a drop is offered only when every selected picture gained the tag.** §9 scopes the
  undo to the dry run's `added` *set*, but the dry run reports a **count**, not ids, and no endpoint
  returns "the selection minus the pictures already carrying this tag". When some pictures already
  had it, the toast says so instead of offering a removal that would over-strip.
- **The announced decoration is parked on `incoming_shares.sender_tag_meta` (one JSONB column).**
  §10.1 wants the seed at *accept*, but the decoration arrives at *announce*. Seeding
  `tag_metadata` straight from the announce would re-seed a recipient who had reset their metadata,
  which §10.1 forbids. One JSONB column keeps seed-on-first-accept exact and follows the house
  precedent for side payloads.
- **`ShareAnnounce` keeps `VERSION = 1`.** `tag_meta` is additive and `#[serde(default)]`, matching
  how feature 10 added `allow_exif_edit`; `check_version` is strict equality, so a bump would reject
  every peer that predates the field.
- **The root entry is always returned, with zero counts**, rather than "no counts": one uniform item
  shape, and the client never has to invent a root node. Root counts are meaningless either way.
- **Collection `MOVE` now returns `405`.** §8 keeps it out of scope and forbids a Finder rename from
  writing `webdav_dir_name`; removing the pending-dir sidecar therefore removes the old
  rename-the-marker path. A folder keeps the name `MKCOL` minted. Noted on the roadmap.
- **CRLF in a description is normalised to `\n`** rather than rejected as a control character, so a
  paste from a Windows client is not an error.
- **Reorder mode uses up/down buttons on desktop too**, not the drag handles §7 sketches: the same
  rows are already photo drop targets, and a second drag system on them would be ambiguous.

### 16.1 Deferred (stored and served, no UI yet)

- ~~**Cover picker** (§6)~~ — shipped with feature 35, whose subtag blocks are the first surface that
  renders a cover. Not a picker in the end: a second picture browser nested in the tag dialog was the
  wrong shape, so the cover is **set from the photo** — the selection panel's `⋯` → *Set as thumbnail
  for &lt;tag&gt;*, offered for the view root when the picture is owned — and the dialog shows a
  preview + Clear (`components/tags/CoverPicker.tsx`).
- ~~**`view_mode`, `grouping`, `subtag_placement`, `children_order`**~~ — all four now have controls:
  the grid's **View** dropdown and merged Sort + Group by menu (feature 35 §4), and a *Sort subtags
  by* submenu on the tag tree for `children_order`.
- **Share popover: unmap a share-mapping target** (§10) — the popover covers recipients, status,
  revoke, public links and "share with someone else", but not the `tagging_services` JSONB
  containment lookup that finds a mapping's sender.
- **Shares tabs** (§10, last paragraph) — the pending-count nav badge and the regrouped tabs.
