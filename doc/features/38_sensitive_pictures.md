# Feature 38: Sensitive pictures

## 0. Status

Spec only — nothing built.

## 1. Overview & goals

Some pictures should not appear on screen by accident: when browsing with someone next to you,
presenting, or scrolling a shared screen. Goal: let a user mark pictures **sensitive**, have the web
viewer **veil** them by default (blurred, redacted or hidden), and reveal them deliberately — one
picture at a time, or all of them for a few minutes in the current tab.

**This is a curtain, not a lock.** Sensitive pictures are still served by the API, presigned, shared
and listed over WebDAV. Nothing here is access control, and the UI copy must never suggest
otherwise — which is also why the name is *sensitive*, not *private*.

Out of scope: encryption, propagating the flag to share recipients, blocking shares, WebDAV (§2.1).

## 2. Decisions (settled)

- **A flag on tag metadata, not a picture attribute.** `tag_metadata.sensitive` (feature 34). A
  picture is sensitive iff any of its stored tags — any source, so rules, segments, share mappings
  and `SharedToMe` nodes count — is at or under a flagged tag. Marking a picture sensitive is
  ordinary tagging; rules, batch edit, WebDAV drop nodes and share mappings mark pictures with no new
  mechanism. A `pictures` column would need its own rule output, hierarchy predicate, filter, batch
  field and WebDAV write path — the second dependency 34 §2 warns about.
- **No reserved tag.** Any subtree can be flagged (`/Photos/Medical`, a `SharedToMe` node, a
  user-made `/Sensitive`), so nothing collides with existing tags and nothing must be re-tagged.
- **Sensitive wins.** Flags inherit downward and cannot be cleared below a flagged ancestor.
- **The one engine-side read of `tag_metadata`.** The picture listing derives `sensitive` and accepts
  a `sensitive` presence filter (§5). Pipeline, hierarchy resolver, sharing and federation still never
  read it — 34 §2 gains this as its second named exception.
- **The display mode is a backend setting; reveals are tab-only memory.** `user_settings` carries the
  mode (§3); per-picture reveals and the show-all override live in a non-persisted store, so a reload
  or a new tab re-veils.
- **Hidden: the client decides, the backend filters** (§5.1). Filtering must be server-side so page
  totals, timeline sections and batch selections (feature 14) agree with the screen — *select all →
  trash* never touches a picture you could not see. Deciding must be client-side, because the
  show-all override exists only in the tab. The backend never reads `user_settings` for this.
- **No `thumbnail_url` withholding.** The client simply does not load the URL of a veiled picture
  (§4). Withholding it server-side would cost a presign round-trip per reveal and protect nothing a
  curtain is meant to protect.

### 2.1 Rejected / postponed

- **WebDAV — postponed** to a separate feature. Until then a hierarchy can `exclude` a flagged tag by
  hand, with the 05 §5.4 caveat that a picture also tagged outside it still surfaces there.
- **Encryption.** Workers need plaintext (thumbnails, EXIF, `content_hash` dedup), recipients fetch
  blobs straight from the owner's backend via presigned URLs, and WebDAV serves bytes. End-to-end
  encryption would move thumbnailing client-side and break dedup and EXIF rules; server-held keys
  only defend against an S3 compromise, which MinIO SSE already covers at the infrastructure level.
- **Propagation to recipients**, including seeding the recipient's `SharedToMe` node from a flagged
  shared tag (34 §10.1). Per-picture propagation would need a picture-level flag on received rows;
  seeding alone is too weak to be worth a protocol field. A recipient flags their own `SharedToMe`
  node instead.
- **Disabling shares of sensitive pictures.** Too restrictive for a sharing-first product; shares
  warn instead (§7).
- **Re-authentication to reveal.** Pointless for a curtain: the same session can fetch the URLs.
- **A one-click "Mark sensitive" toggle on a picture.** Marking is tagging; a toggle would need a
  designated tag and a "sensitive via another tag" state it cannot undo.

## 3. Display modes

`user_settings.sensitive_display_mode`: `visible` | `blurred` (default) | `redacted` | `hidden`.

| Mode         | Grid tile                         | Single-picture surfaces (lightbox, cover, …) | Listings            |
|:-------------|:----------------------------------|:---------------------------------------------|:--------------------|
| **Visible**  | image                             | image                                        | all                 |
| **Blurred**  | blurhash (Redacted if none yet)   | blurhash + Reveal                            | all                 |
| **Redacted** | dark patterned tile + lock icon   | same + Reveal                                | all                 |
| **Hidden**   | absent                            | Redacted + Reveal                            | `sensitive=missing` |

Redacted exists because a blurhash still leaks the colour layout. A sensitive picture always carries
the **sensitive marker** (red ring + small lock badge), in every state including revealed and
Visible, so it stays identifiable at a glance.

### 3.1 Render precedence

For one sensitive picture, first match wins:

1. revealed individually in this tab → **visible**
2. show-all override active in this tab → **visible**
3. mode `visible` → **visible**
4. mode `blurred` → **blurred**
5. mode `redacted` / `hidden` → **redacted**

A veiled picture never mounts its `<img>`/player, so its thumbnail is neither fetched nor cached until
revealed.

### 3.2 Hidden inside a flagged tag

A listing whose primary tag scope (the gallery `tag` param, feature 35 §7) is at or under a flagged
tag does **not** send `sensitive=missing` and renders those pictures **Redacted** — opening a flagged
tag must not show an empty view. The client decides this from the tag tree it already holds
(`GET /tags` carries `meta.sensitive`). Hierarchy-directory listings have no primary tag and keep
Hidden. Anywhere a single sensitive picture still renders under Hidden (a cover, a direct link, a
flagged-tag view), it renders Redacted.

## 4. Tab-scoped reveals

A small `sensitive` Zustand store, **not persisted**:

- `revealed: Set<pictureId>` — per-picture reveals for the tab's lifetime. The lightbox button
  toggles **Reveal / Hide again**.
- `showAllUntil: number | null` — the show-all override. Started from the lightbox of a veiled
  picture or from Settings ("Show sensitive pictures in this tab for [10 min ▾]", presets
  5/10/30/60; not offered in Visible mode). While active, the **TopBar** shows a countdown chip with
  a **Stop** button; expiry or Stop clears it, the Hidden-mode listings refetch (the `sensitive`
  param is part of the query key) and everything re-veils except individual reveals.

## 5. Derivation & filter (backend)

Per request, load the caller's flagged paths once
(`SELECT tag_path FROM tag_metadata WHERE user_id = $1 AND sensitive` — a PK-prefix scan) and bind
them as `ltree[]`:

```sql
EXISTS (SELECT 1 FROM tags t WHERE t.picture_id = p.id AND t.tag_path <@ $flagged)  -- ltree <@ ltree[]
```

- **Derived field.** `sensitive: bool` on `PictureListItem` and `PictureDetail`, like `has_gps`. An
  empty `$flagged` short-circuits to `false` with no subquery.
- **Filter.** `ScopeParams.sensitive: PresenceFilter` (`any` default | `present` | `missing`) → a
  `PictureListFilter.sensitive` arm, AND-composed like `gps`. Shared by the flat and hierarchy filter
  kinds, so `GET /pictures`, every batch endpoint and the share-warning count (§7) speak it.
- **Whose flags.** Always the user holding the rows: the caller for their own and received pictures,
  the share **owner** for public-share listings (§7.1).

### 5.1 Who filters in Hidden mode

The backend applies `sensitive=missing` only when the request carries it; it is a plain, stateless
filter, never inferred from the user's mode. The client sends it when its **effective** mode is
Hidden — the stored setting, minus the tab's show-all override, minus the §3.2 flagged-scope
exception. The backend's only extra read is the flagged-path list above, which the derived field
needs anyway.

## 6. Frontend surfaces

- **One veil, in the shared image path.** The veil and the marker live where `OrientedImage` already
  falls back to the blurhash when `src` is absent, plus a Redacted tile. Every picture render inherits
  it: `PhotoCard`, `Lightbox`/`LightboxCarousel`, `CoverThumb` (the URL query is gated on the detail's
  `sensitive`), `SelectionPanel`, the fix panels, and the public components. Videos do not mount the
  player while veiled.
- **Gallery params.** `sensitive=missing` is derived state, not a URL param: it is injected where
  `useGalleryParams` turns params into listing filters, so `useTimelineView`'s `selectionFilter`
  carries it too (§5.1).
- **Details panel.** A "Sensitive — via /Sensitive, /Photos/Medical" row, computed client-side from
  the detail's tags and the tag-tree metadata.
- **Edit tag dialog.** A *Sensitive* checkbox with curtain help text; shown checked-and-disabled
  ("inherited from /Sensitive") under a flagged ancestor. *Reset metadata*'s consequence wording
  mentions the flag.
- **Tag tree.** Lock icon on flagged tags; counts unchanged (they are membership, §9).
- **Settings page.** The mode selector (with the account section's explicit Save), and the
  show-all button beneath it, which is never saved.

## 7. Sharing

Shares are unchanged: coverage is by tag, sensitive pictures in coverage are announced and shown
normally to recipients.

- **Creation warning.** The outgoing-share and public-share dialogs count
  `GET /pictures?tag=X&sensitive=present&page_size=1` → `total` and, when non-zero, warn: "N sensitive
  pictures in this tag — recipients will see them normally" (public: "visitors will see them
  blurred"). No warning for pictures flagged after the share exists.

### 7.1 Public shares

Visitors see the owner's sensitive pictures **Blurred** with per-picture reveal and the same show-all
override in the public header; the owner's own display mode does not apply, and Hidden never alters
coverage. The flag is computed against the owner's tags on the public listing and detail, so it is a
local read with no protocol change (27 §10's received pictures included — the share owner's rows).

## 8. Schema & API

```sql
ALTER TABLE tag_metadata ADD COLUMN sensitive boolean NOT NULL DEFAULT false;
CREATE TYPE sensitive_display_mode AS ENUM ('visible', 'blurred', 'redacted', 'hidden');
ALTER TABLE user_settings
    ADD COLUMN sensitive_display_mode sensitive_display_mode NOT NULL DEFAULT 'blurred';
```

`sensitive = false` is the default, so 34's prune-if-all-default rule needs no special case.

- `GET /tags` `meta.sensitive`; `PUT /tags/meta` accepts `sensitive`.
- `GET`/`PATCH /settings` gain `sensitive_display_mode`.
- `PictureListItem.sensitive`, `PictureDetail.sensitive` — authenticated and public endpoints.
- `sensitive=any|present|missing` on `GET /pictures` and in `ScopeParams` (selections, aggregates).

## 9. Edge cases

- **Hidden mode counts.** Tag-tree counts are membership and include sensitive pictures; a Hidden-mode
  listing can therefore show fewer pictures than the tree count. Accepted.
- **Explicit ids.** `include_ids` in a selection are never filtered by `sensitive` — the user picked
  them.
- **Rename cascade** moves the flag with its row (34 §12); deleting the tag's metadata clears it.
- **Unprocessed picture** (no blurhash yet) in Blurred mode renders Redacted.
- **Override expiry with the lightbox open** on a sensitive picture re-veils it unless individually
  revealed.
- **Download / Save a copy** from a veiled lightbox is allowed — an explicit action, not an accident.
- **Received pictures** follow only the recipient's own flags; the owner's flags never travel.

## 10. Doc updates

- [ ] `01_GENERAL_SPECIFICATIONS.md §1` — tag metadata can mark a subtree sensitive (viewer curtain).
- [ ] `features/34_tag_metadata.md §2` — the second named exception (listing derivation + filter).
- [ ] `06_API_REFERENCE.md` — §6.1 settings, §6.3 `sensitive` field + filter, §6.7 tag meta,
  public-share listing/detail.
- [ ] `05_FRONTEND_ARCHITECTURE.md` — §5 the `sensitive` store and derived param, §7 the veil, TopBar
  chip, Settings, Edit tag dialog.
- [ ] `features/27_public_shares.md §15` — veiled sensitive pictures on public pages.
- [ ] `99_ROADMAP.md` — entry moved to Done; WebDAV filtering stays under Later.

## 11. Work breakdown

1. [ ] Migration — `tag_metadata.sensitive`, `sensitive_display_mode` enum, `user_settings` column.
2. [ ] `TagMetadataRepository` — the field in upsert/list; `flagged_paths(user_id)`.
3. [ ] Listing — flagged paths once per request, derived `sensitive` on list item and detail,
   `ScopeParams.sensitive` → `PictureListFilter` arm; public listing/detail against the owner.
4. [ ] Settings API field; `GET`/`PUT` tag meta field.
5. [ ] Front: `sensitive` store, render-precedence hook, veil + Redacted tile + marker in the shared
   image path; `CoverThumb` gating; video guard.
6. [ ] Front: effective-mode `sensitive=missing` in the gallery/selection filter builder with the
   flagged-scope exception (§3.2, §5.1).
7. [ ] Front: lightbox Reveal / Hide again + show-all start; TopBar countdown chip; Settings mode +
   show-all; details-panel "via" row.
8. [ ] Front: Edit tag dialog checkbox (inherited state, reset wording); tag-tree lock icon.
9. [ ] Front: share-dialog warnings; public pages (Blurred default, override in header).
10. [ ] Tests: derivation (ancestor flag, rule/`incoming_share` sources, a flagged `SharedToMe`
    node), `sensitive` filter through selection resolution, prune-if-default with the new field,
    public listing flagged against the owner.
