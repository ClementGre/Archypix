# Timeline view

## 0. Status

Shipped. Built on feature 34's app-start tag payload, `lib/tagTree.ts` resolution helpers and the
§12.7 write queue — see **34 §16** for what that feature deferred to this one (`view_mode`,
`grouping`, `subtag_placement` and the cover, all stored and served but with no UI until now).
Deviations taken here are in **§13**.

## 1. Overview & goals

Hierarchical tags are the primary navigation method, and they stop being ergonomic once a library has
hundreds of events: the grid is a uniform date-sorted wall, and the only structure visible is whatever
the user encoded into slug prefixes.

This feature turns the gallery into a **structured browse** driven by the tag subtree of wherever the
user is standing, using the names, dates, covers and ordering from
[34_tag_metadata.md](34_tag_metadata.md). It also **rewrites the gallery's tag-filter param model**,
which has accumulated four overlapping ways to say the same thing (§7).

Nothing here is specific to a namespace. There is no `Era` in the code: the view works identically at
any tag with subtags, so a photographer pointing it at `Clients` gets the same behaviour — and
identically at the root, which is just the tag `''` (feature 34 §3.3).

Depends on: **34** (display names, dates, per-tag view preferences, ordering, the root row).

## 2. Decisions (settled)

- **One rule, no modes per depth (§3).** Rendering recurses; "depth" is how far the user has expanded,
  not a setting. This is what keeps it working at the root, `Era`, `Era/2026`, and `Era/2026/Vietnam`
  without special cases.
- **The root is a tag.** The no-tag gallery view reads its `view_mode` / `grouping` /
  `children_order` / `subtag_placement` from the `tag_path = ''` row (34 §3.3), so it needs no
  branch of its own. `direct` at the root means **untagged** — `TagPredicate.untagged` already exists.
- **Sections absorb the chronology (§5).** Subtag blocks sit at the top of the grouping section their
  date falls into, rather than being interleaved photo-by-photo. Photo-level interleaving was
  rejected: expansion would have to split the parent flexbox mid-flow, and block placement would
  depend on the pagination frontier.
- **Block placement is a stored option, not a derived special case.** `subtag_placement` (34 §3.4) is
  `top` or `in_sections`, defaulting to `in_sections` for a date sort at a nested tag and `top`
  everywhere else — including the root, whose children are namespaces spanning every month. An earlier
  draft hid this as an implicit rule for non-date sorts; making it an option costs one nullable column
  and gains "month-grouped photos with my subtags pinned above", which was previously inexpressible.
- **Sections derive from direct photos.** No direct photos ⇒ no sections ⇒ subtags simply list. This
  is not a special case for the top of a hierarchy; it falls out, and it is exactly the behaviour
  wanted at a namespace root whose children are all subtags.
- **Subtag is the default content mode.** With no children, all three modes render identically, so
  the default is safe everywhere.
- **`direct` is the plain `exact` predicate**, i.e. "carries T itself" — not "T is its deepest tag".
  Manual assignment already prunes redundant manual ancestors (`assign_inner`'s cleanup CTE), so the
  two coincide for hand-tagged pictures. They diverge only across sources (§10.12), which is rare
  enough not to justify putting `minus_children` on the wire.
- **View preferences live in `tag_metadata`, not the URL.** They follow the user across devices and
  arrive with the app-start payload. The cost is that view mode is not shareable via a link — accepted,
  since the usual second reader is the same user on another device — so the header states the active
  mode whenever it is not the default (§4).
- **Multi-exact is dropped.** The per-tag `(=)` toggle and the `exa[]` param go away; *Direct only* in
  the content dropdown covers the real use. Holding several exact tags at once was an advanced filter
  with no surface that needed it.
- **Grouping and fix mode are mutually exclusive, and fix mode also pins the content mode.** §9.

## 3. The rule

> At any tag **T** (the root being `T = ''`), render T's **direct** (exact-match) photos and T's
> **child tags** as one stream. A child renders as a collapsed block; expanding it applies the same
> rule at that child, using **that child's own** remembered content mode, grouping and placement.

At the root, "direct photos" are the **untagged** ones and "child tags" are the top-level tags.

## 4. Controls

Two per-tag controls, both persisted on `tag_metadata` (34 §3).

**View** (`view_mode`) — a dropdown in the grid header labelled **View** (*Recursive view mode* in
full), carrying exactly three options. It replaces the tag tree's `(=)` exact toggle and the
`TagFilterBar` chip's include↔exact switch, a less fiddly home than a per-node icon.

- `direct` — only photos carrying T itself (at the root: untagged photos).
- `all` — every photo under T, flat. Today's default behaviour.
- `subtag` — §5. **Default.**

Because the setting is sticky and invisible in the URL, the header shows it explicitly with a
one-click reset whenever it is not `subtag` — otherwise someone who set *Direct only* on a tag months
ago lands on a near-empty grid with no explanation.

**Sort + Group by** (`grouping`) — **one two-column dropdown**, replacing today's `SortMenu`: the left
column picks the sort field, the right column the bucketing **for that field**. They merge because the
dependency is structural — a bucket must be a contiguous run in the sorted order, or the same bucket
reappears down the page — so a dependent second column states the constraint instead of leaving two
sibling menus free to disagree. Buckets are remembered **per sort field** (34 §3.1), so switching sort
and back does not lose the setting. On mobile the two columns stack, the bucket column disclosing
under the chosen field.

| Sort field | Buckets | Default |
|---|---|---|
| `captured_at`, `ingested_at`, `updated_at` | `none` · `year` · `quarter` · `season` · `month` | `month` |
| `filename` | `none` · `prefix` (first N chars, N input 1–16) | `none` |
| `file_size` | `none` · `magnitude` — <100 KB, 100 KB–1 MB, 1–10 MB, 10–100 MB, 100 MB–1 GB, >1 GB | `none` |
| `geo_near` | `none` · `magnitude` — <10 m, 10–100 m, 100 m–1 km, 1–10 km, 10–100 km, 100–1000 km, >1000 km | `none` |
| `time_near` | `none` · `magnitude` — <1 h, 1–6 h, 6–24 h, 1–7 d, 1–4 wk, 1–12 mo, >1 yr | `none` |

`magnitude` is one `kind` with a per-field ladder — byte decades, metric distance decades, and a
calendar-ish time ladder — rather than three kinds, so a field gains bucketing by declaring its ladder.

**Null values get a terminal bucket.** A photo with no `captured_at` under month grouping, no
`filename`, or no `file_size` lands in a single named section — *No date*, *No filename*, *Unknown
size* — rendered last (or first, wherever the backend's null ordering puts it; `undatedFirst` already
floats undated pictures under date-fix mode). The rows are contiguous either way, so the bucket is a
run like any other. Without it a third of an imported camera roll silently falls out of every section.

Under the proximity sorts it buckets the **absolute** delta from the reference, so "3 h before" and
"3 h after" share a section; the sort itself is already nearest-first and direction-blind (feature 29
§6). Season buckets read `user_settings.hemisphere` (34 §3) — the viewer's convention, applied
regardless of where the photo was taken. `prefix` labels its sections by the literal prefix (`IMG`,
`DSC`, `PXL`), which is what makes a camera-roll import legible precisely when date metadata is the
thing that is missing.

## 5. Rendering model

At T in `subtag` mode:

1. T's direct photos are partitioned into **grouping sections** (month by default).
2. Child tags are placed per `subtag_placement` (34 §3.4):
   - `top` — all blocks render once, before the first section.
   - `in_sections` — each child lands in the section its date falls into. Available only under a date
     sort; under any other sort the resolver falls back to `top`, because a tag has no filename, size
     or distance.
3. Within a section: **subtag blocks first, then that section's direct photos.**
4. Expanding a block opens full-width immediately after the block row — predictable, because blocks
   are contiguous at the top of each section.
5. An expanded child renders by its own `view_mode` + `grouping` + `subtag_placement`.

**Which date places a child** under `in_sections`: the endpoint that comes **first in the active sort
order** — `date_to` under descending (newest first), `date_from` under ascending. A tag spanning
Jul 28 – Aug 3 therefore appears at the top of August in the default newest-first view, which is where
the eye looks for it, and in July when reading forwards.

```
── August 2026 ─────────────────────────────
   [▣ Vietnam 🇻🇳  Aug 1–14 · 184]  [▣ Corsica  Aug 22–26 · 61]
   ▦ ▦ ▦ ▦ ▦ ▦ ▦ ▦ ▦ ▦ ▦ ▦
── July 2026 ───────────────────────────────
   ▦ ▦ ▦ ▦ ▦
```

**Collapsed block** — a card in the flex flow, photo-sized, cover + display name + count + date range.
**Expanded block** — breaks to a full-width inline section with its own header, recursing §3.

**Child ordering** within a section follows the parent's `children_order` (34 §7). Children with no
date (no photos, no override) flush at the end of the stream.

**Auto-expansion.** When `exact_count === 0` there is nothing to interleave with, so children expand
by default. Each expanded section's query is gated behind an `IntersectionObserver` — the same pattern
as the existing pagination sentinel in `PhotoGrid` — so a tag with fifty children does not fire fifty
requests. That makes expand-all safe without a count threshold. Sections that scroll far out of view
release their pages back to the query cache and keep only their registration (§8), so a long
expand-all does not accumulate fifty live infinite queries and their thumbnails.

**Under active cross-cutting filters** (`inc`/`exc`, §7) a block's **count is hidden**: counts come
from the unfiltered tag payload and would be wrong. Date ranges are kept — they describe the tag, not
the result set. The block itself still renders, because knowing whether it is empty would cost a query
per collapsed block and defeat the zero-request design; expanding one that matches nothing shows *"No
photos match the current filters"* inside the section rather than an empty void.

**Section counts.** A grouping section header carries no count while its stream is still paginating —
the section is a slice of one paginated query (§6), so a count would be wrong until the last page
lands. It appears once the query reports no further pages.

## 6. Data sources

No new endpoint. Three sources, all existing:

| Needs | Source |
|---|---|
| child list, counts, dates, covers, order, names | app-start tag payload (34 §4), cached |
| T's direct photos | `usePictures({ ...filters, exact: T })` |
| an expanded child's photos | the same hook scoped to that child |

`PictureListItem` carries **no tags** (`front/src/lib/types.ts`), so partitioning one
descendant-inclusive query client-side is impossible without widening the list payload. Per-section
queries avoid that entirely, and give lazy loading for free: a collapsed block costs zero requests.

**Two kinds of "section", one word.** They paginate differently and the distinction matters:

- A **grouping section** (month, size decade, …) is a client-side slice of **one** query — T's direct
  photos. It does not paginate independently; the stream pages as a whole and sections grow as rows
  arrive. A section straddling a page boundary simply gains rows in place.
- An **expanded child section** is its own infinite query, mounted lazily behind the visibility gate,
  and it has its own grouping sections inside it.

**Payload additions.** Grouping is computed entirely client-side, so every bucketed field must be
readable on the list item. Two are missing: `file_size` and `updated_at` — both already valid
`SortField`s, neither on `PictureListItem`. Add `file_size: number | null` and `updated_at: string` to
the list item and its backend row mapping. The other ladders need nothing new: `geo_near` already ships
`distance_m` under that sort (feature 29 §6), and `time_near` is derivable from `captured_at` plus the
`near_time` param the client already holds.

## 7. Filter/query refactor

The gallery currently has **four** overlapping tag params — `tag`, `inc[]`, `exc[]`, `exa[]` — all
folded into one `TagPredicate`. With per-section queries and multi-exact dropped, the model collapses
to:

| Param | Role |
|---|---|
| `tag` | the **view root**: the tag whose subtree structures the view (absent ⇒ the root, `''`) |
| `inc[]` | cross-cutting AND filters applied to **every** query in the view |
| `exc[]` | cross-cutting NOT filters, likewise |
| ~~`exa[]`~~ | **removed** — derived per section, never user state |

Per-section scoping is internal, not URL state: rendering a section issues `exact: <section tag>`
itself. The URL stays short regardless of how many groups are expanded.

**The backend needs no new predicate capability.** `TagPredicate` already carries `include` / `exact` /
`exclude` / `match_all` / `untagged`, and `render_predicate` already renders them; every leaf query in
every view mode is `exact: [X]`, `include: [X]`, or `untagged` at the root. The backend change is
limited to making the wire `exact` single-valued in `build_flat_predicate`. The rewrite is a frontend
one.

**`selectionFilter` follows the content mode**, always layered with `inc`/`exc`:

| Content mode | `selectionFilter` at T | at the root |
|---|---|---|
| `direct` | `exact: [T]` | `untagged: true` |
| `subtag` | `include: [T]` | no tag arm (everything) |
| `all` | `include: [T]` | no tag arm (everything) |

`subtag` and `all` share a filter, so toggling between them **preserves the selection**; switching to
or from `direct` changes what the view contains and correctly clears it via the existing `filterSig`
effect in `PhotoGrid`.

**⌘A selects the subtree, not the visible set.** In `subtag` mode `include: [T]` covers photos inside
collapsed blocks that are not on screen. Matching the visible set instead would mean a selection that
changes as the user expands, and it cannot be expressed as one `PictureFilter` — which is the whole
point of the feature-14 selection descriptor. So the behaviour stays, and the selection status bar
says so: *"184 selected — includes 3 collapsed groups"*.

## 8. Flat visible order

Three consumers need a single ordered array and all three are load-bearing:

| Consumer | Purpose |
|---|---|
| `orderedIds` → `selectTo` | shift-click range selection |
| `useGridItems` | feature 30 §5.2 grid-local GPS anchor scan |
| `Lightbox items` + `loadMore` | carousel navigation and paging |

With nested sections there is no longer one array. A `GroupedGridContext` provider collects
registrations from each mounted section — `(sectionKey, renderIndex, items, fetchNextPage, hasNextPage)`
— and exposes the flattened **visible render order**, which then feeds all three. Sections register on
mount and on data change, and unregister on collapse.

The registration carries `fetchNextPage` because ordering alone is not enough: paging past the last
item of section *k* has to advance **that section's** query, not a global one. The context's
`loadMore` resolves the owning section from the current index and calls its fetcher, falling through to
the next section when the current one is exhausted.

Shift-click therefore spans groups, selecting everything between in visual order, and the lightbox
continues from the end of one group into the next. Visual order is the least surprising rule for both.

This is the riskiest part of the feature, which is the direct reason for §9.

## 9. Fix mode

While `params.fix` is set, the view is pinned flat: **grouping forced to `none` and `view_mode` forced
to `all`**. The fix entry points are disabled while a grouped view is active, each showing why. The two
are semantically opposed: fix mode wants a flat date-sorted stream to find temporal and spatial
neighbours, grouping wants structure.

Forcing `view_mode` matters as much as forcing grouping, and for the same reason. Today `tag=X` is an
*include*, so the grid holds X's whole subtree; under this feature the default is `subtag`, which holds
only direct photos. Leaving it alone would silently strip the grid-local GPS anchor scan
(`useFixAnchors`) of most of its neighbours and push it onto the directed-bracketing fallback — a
quiet degradation, not a visible one. With both pinned, `useGridItems` keeps seeing exactly today's
flat array and feature 30's GPS interpolation and reference-picking are **untouched** by §8.

Both overrides are shown in the header with their reason, and both restore on leaving fix mode —
neither is written to `tag_metadata`.

Allowing grouping and fix together — by force-expanding all groups, or by scanning visible items only
and accepting degraded interpolation — is a possible later feature, deliberately not this one.

## 10. Edge cases

1. **T has no children.** All three content modes render identically; the dropdown still shows, with
   `subtag` and `all` indistinguishable. No special-casing.
2. **T has children but no direct photos.** No sections (§2); children list in `children_order`, and
   auto-expand applies. This is the namespace-root case.
3. **A child whose date range spans several sections.** Under `in_sections` it is placed by the
   sort-leading endpoint (§5) and appears once. It is a block, not a set of photos — it does not
   fragment.
4. **A child with no date at all.** Flushed at the end of the stream, after the last section.
5. **Non-date sort.** Every sort field has buckets (§4), so sections exist throughout; blocks fall back
   to `top` whatever `subtag_placement` says, since a tag has no filename, size or distance.
6. **A tag's remembered grouping has no entry for the active sort field.** That field's default
   applies (`month` for dates, `none` otherwise) and nothing is written until the user picks one —
   consistent with 34's prune-if-all-default rule.
7. **A grouping write lands after the 5-minute re-fetch.** `PUT /tags/meta` is a partial upsert
   carrying only changed fields (34 §4.1), so it cannot clobber a concurrent change from another
   device.
8. **Trash filter `include` / `only`.** Structure survives: the tag payload carries trashed counts and
   ranges alongside the live ones (34 §4), so a tag whose photos are all trashed still renders as a
   block in the trash view — and is hidden in the default view. Blocks show the count matching the
   active trash filter, and hide it under `inc`/`exc` for the §5 reason.
9. **Hierarchy browse mode** (`params.hierarchy` set) keeps today's flat rendering — hierarchies have
   their own directory structure and the two must not compete. The View dropdown hides; the merged
   Sort menu keeps its sort column and drops the bucket column.
10. **Deep expansion.** No depth cap; each level is a mounted component with its own lazy query.
    Practically bounded by how deep a user's tags go, by the visibility gate, and by the page release
    in §5.
11. **Expansion state is transient** — a local store, not the URL, and not `tag_metadata`. Five
    expanded groups would bloat a link, and expansion is not a preference.
12. **A picture in both T's direct photos and a child's block.** Possible when the two tags come from
    different sources — a rule or segmentation service asserting `Era.2026` while the user manually
    tagged `Era.2026.Vietnam`, or one template emitting two levels. `assign_inner` prunes redundant
    ancestors only within `source = 'manual'`, so both rows survive and `exact: Era.2026` matches.
    Accepted (§2): the picture shows twice. Suppressing it would mean putting `minus_children` on the
    wire for a case hand-tagging never produces.
13. **A child block that is empty under the active filters.** Rendered anyway, with *"No photos match
    the current filters"* on expansion (§5). Hiding it would cost one query per collapsed block.

## 11. Doc updates

- [x] `05_FRONTEND_ARCHITECTURE.md §8` — a *Structured browse* subsection (the recursive rule, the
  bucketers, block placement and laziness, the `GroupedGridContext`, the fix-mode pin); §5's param
  table and §7's tag-tree/`TagFilterBar`/toolbar entries for the reduced param model, the **View**
  dropdown and the merged Sort + Group by menu.
- [x] `06_API_REFERENCE.md §6.3` — `exact` is single-valued on `GET /pictures`; `file_size` and
  `updated_at` added to `PictureListItem`.
- [x] `features/29_query_proximity_and_missing_filter.md §6` — `distance_m` now also feeds `geo_near`
  bucketing.
- [x] `features/30_photos_fix_tools.md §3` — the grouping **and content-mode** exclusivity (§9).
- [x] `features/34_tag_metadata.md §16.1` — the deferred view controls and cover picker now exist.
- [x] `99_ROADMAP.md` — entry moved to Done.

## 12. Work breakdown

1. [x] `useGalleryParams` rewrite: `tag` + `inc[]` + `exc[]`; `exa[]` dropped (and the `TagFilterBar`
   include↔exact chip toggle); `selectionFilter` moved to `useTimelineView`, per content mode, root
   included (§7).
2. [x] Backend: `build_flat_predicate` takes a single `exact`; `file_size` and `updated_at` added to
   `PictureListItem` and its row mapping (§6).
3. [x] `GroupedGridContext` — registration with per-section `fetchNextPage`, flattened visible order,
   and `selectTo` / `useGridItems` / `Lightbox.loadMore` rewired onto it (§8).
4. [x] Grouping engine (`lib/grouping.ts`), one bucketer per sort field, each with its null bucket;
   children placed by the sort-leading endpoint, undated flushed, `subtag_placement` honoured
   (`lib/timeline.ts`).
5. [x] `SubtagBlock` (collapsed card) and the recursive `TagStream` with the `IntersectionObserver`
   load gate.
6. [x] **View** dropdown with the non-default indicator + reset; `SortMenu` merged into the
   two-column Sort + Group by menu; the `(=)` control removed from `TagTree`.
7. [x] `view_mode` / `grouping` / `subtag_placement` persisted through the 34 §4.1 write queue, root
   row included.
8. [x] Fix-mode exclusivity guards for grouping **and** content mode, with reasons (§9, §13).
9. [x] Tests: `build_flat_predicate`'s single-valued `exact` (and that `untagged` still rejects a tag
   arm) as unit tests in `services/pictures.rs`. The rendering rules are frontend-only and the
   frontend has no test runner (34 §15.12), so they are covered by the build.

## 13. Deviations taken while implementing

- **Fix mode pins the view; it does not disable the fix entry points.** §9 says "the fix entry points
  are disabled while a grouped view is active", but grouping defaults to `month` for every date sort,
  so that would make fix mode unreachable from a default view. Both overrides are applied and stated
  in a header strip instead, and the exclusivity is enforced the other way: grouping is forced off and
  the Sort menu's bucket column disappears while `fix` is set.
- **Fix mode rules out `subtag` only, not `direct`.** §9 forces `view_mode` to `all`, but its reason —
  `useFixAnchors` needs a flat stream of neighbours — is satisfied by `direct` too, and narrowing to
  one tag's own photos is often exactly the set being fixed. Only `subtag` is unavailable, and it is
  the stored value that comes back on exit.
- **A stored view is not flagged as "non-default".** §4 wants the header to state a non-`subtag` mode
  with a one-click reset. Once the mode genuinely persists, highlighting it and offering an X reads as
  an anomaly to undo rather than a preference. The **View** trigger simply names the active mode, and
  the Sort trigger keeps its highlight only for a proximity sort, which *is* transient URL state.
- **An expanded block replaces its tile** instead of staying in the row with its stream spliced
  underneath (§5's "opens full-width immediately after the block row"). Leaving the tile in place made
  the panel look like a second, separate object; the same card widened reads as one thing opening. A
  corner **↗** covers the other intent — browsing *into* the tag as the view root.
- **Blocks lay out on a grid, not the photo flex row.** §5 puts blocks "in the same flow as the
  photos". Growing them like photos made a short last row wider than the rows above it; an `auto-fill`
  track per tile keeps every block the same size, which is the point of a block having no aspect ratio.
- **Headers stack instead of overlapping.** §5 only asks for sticky section headers. With the
  recursion those pin at the same offset and cover each other, so every level is handed the
  accumulated height of the levels above it and z-index decreases with depth. This is also why
  spacing is a `gap` rather than a margin (a margin offsets where a sticky header pins), why a header
  bleeds out of its container's horizontal padding (inset by it, a pinned header reads as a floating
  bar with the page down either side), why the grid's padding moved to a wrapper *inside* the scroll
  container (padding on the scrollport itself holds a pinned header off the top), and why an expanded
  card cannot carry `overflow-hidden` (it would become the scrollport of its own headers).
- **Child buckets extend the section list.** §5 places a child "in the section its date falls into",
  but §2 derives sections from direct photos — a child dated in a month T has no photos in would have
  nowhere to go. `mergeSections` adds an empty section for each such bucket, in sort order. Date
  bucket keys are zero-padded so key order *is* chronological order.
- **Off-screen sections are not released.** §5/§8 want a scrolled-away section to release its pages
  and keep only its registration. Unmounting the query would churn the flattened order, so expanded
  children simply mount lazily and stay; the visibility gate already keeps a deep expand-all from
  firing every request at once. Revisit if a very deep tree gets heavy.
- **The root never auto-expands.** §5 auto-expands children when `exact_count === 0`, but the root
  entry always comes back with zero counts (34 §16), so reading it would expand every namespace on
  first paint.
- **One rendering path, including the flat view.** `all` + no grouping is the degenerate case of the
  same recursion, so `PhotoGrid` has no separate flat branch — only hierarchy browse (§10.9) keeps
  its own, and it registers as a single section so selection and the lightbox stay uniform.
- **`children_order` gained explicit controls.** 34 §7 only ever switched it to `manual` on entering
  reorder mode. A *Sort subtags by* submenu (custom / display name / tag name / start date / end
  date) now writes it directly — the stored values existed, nothing surfaced them.
- **The cover picker shipped here** rather than staying deferred (34 §16.1): subtag blocks are the
  first thing that renders a cover, so "clear only" would have been visible as a missing feature.
