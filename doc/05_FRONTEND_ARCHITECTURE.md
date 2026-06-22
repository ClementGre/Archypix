# Frontend Architecture

Documentation of the **implemented** Archypix web frontend (`front/`) for developers and AI agents working on it. For the HTTP contract it consumes,
see [06_API_REFERENCE.md](06_API_REFERENCE.md); for product semantics, [01_GENERAL_SPECIFICATIONS.md](01_GENERAL_SPECIFICATIONS.md).

---

## 1. Goals and constraints

- **Pure static SPA** — no SSR, no build-time data fetching. The `dist/` bundle is served from any CDN with an `index.html` fallback for all routes.
  The architecture spec places the frontend as a "static CDN" peer; SSR would add ceremony with no benefit.
- **Agent-friendly** — all component source (including shadcn/ui primitives) lives in `src/`, not `node_modules`. Every component can be read,
  grepped, and edited as a normal project file.
- **Federated by design** — there is no single API base URL. The client resolves, per logged-in user, which backend hosts their identity and talks to
  it directly (see §3).
- **MVP-quality UX** — real loading/empty/error states, dark-first theme, responsive three-pane workspace. Side panels are
  resizable (drag handle, width persisted) on desktop and collapse to overlay drawers on mobile (`< md`); a thin footer status bar
  carries storage/services/view/selection stats and the thumbnail-size slider.

---

## 2. Stack and build

| Concern       | Choice                                                                                  |
|---------------|-----------------------------------------------------------------------------------------|
| UI / language | React 19 + TypeScript (strict: `noUnusedLocals/Parameters`, `verbatimModuleSyntax`)     |
| Bundler       | Vite (`@` → `src` alias in `vite.config.ts` + `tsconfig.app.json`)                      |
| Styling       | Tailwind CSS v4 — CSS-first `@theme` in `src/index.css` (zinc neutral, emerald primary) |
| Components    | shadcn/ui in `src/components/ui/` (Radix primitives, copied source)                     |
| Server state  | TanStack Query v5                                                                       |
| Client state  | Zustand                                                                                 |
| Routing       | React Router v7                                                                         |
| Forms         | React Hook Form + Zod (`src/lib/schemas.ts`)                                            |
| HTTP          | axios                                                                                   |
| Drag & drop   | @dnd-kit (pipeline reordering)                                                          |
| Misc          | blurhash (thumbnail placeholders), sonner (toasts), lucide-react (icons)                |

Commands: `npm run dev` (Vite :5173), `npm run build` (`tsc -b && vite build`). Node 24 / npm. **Theme:** dark is the base `@theme`; light mode is the
`.light` class on `<html>` (toggled by `stores/theme.ts`). The repo runs a code formatter — match surrounding style; don't fight it.

---

## 3. Connection model (federated)

There is **no `VITE_API_BASE_URL`**. Flow:

1. **WebFinger** (`api/webfinger.ts` → `resolveBackendUrl(username, instance)`) queries
   `{scheme}://{instance}/.well-known/webfinger?resource=archypix:@user:instance`
   and returns the `backend_url` link (scheme + host).
2. **Login** (`api/auth.ts` → `login`) resolves the backend, POSTs `/api/auth/login` there, stores
   `{ accessToken, refreshToken, backendUrl, instance }`
   in the auth store, then loads `/api/auth/me`.
3. **Authenticated calls** go through `api/client.ts` → `apiClient` (a single axios instance). Its request interceptor reads `backendUrl` +
   `accessToken`
   from the auth store **at request time**, so every call targets the right instance. The response interceptor does a **single-flight refresh** on 401
   (dedup'd across concurrent 401s); if refresh fails it clears the session and `ProtectedRoute` redirects to `/login`.
4. **Registration** (`api/auth.ts` → `register(payload, domain)`) always `POST`s to `{domain}/api/public/register` — a path served by both a
   standalone
   backend and the resolver (which forwards to a chosen backend), so the frontend never has to know the topology. `domain` defaults to the
   global domain but the register page lets the user target a custom instance (same handle control as login).

**Env** (`.env`, all `VITE_`-prefixed, documented in `.env.example`): `VITE_GLOBAL_DOMAIN`, `VITE_USE_HTTPS`. Resolved in
`lib/constants.ts` (`GLOBAL_DOMAIN`, `USE_HTTPS`, `SCHEME`, `originFor(domain)`). Cross-instance picture fetching relies on dev `CORS_ORIGINS=*`.

The **login and register pages** share the handle as an editable control: `@<username>` + a click-to-edit instance field defaulting to
`GLOBAL_DOMAIN`,
so a user can authenticate or register against any instance. The chosen instance is persisted (`getPreferredInstance`/`setPreferredInstance` in
`lib/constants.ts`, `localStorage` key `archypix_instance`) so the two pages stay in sync. Both render `InstanceCorsWarning`
(`components/common/`), which warns that a custom instance only works if this frontend's URL is in that backend's CORS allowlist.

---

## 4. Routes (`src/App.tsx`)

| Path           | Page                | Auth       | Notes                                                                                                |
|----------------|---------------------|------------|------------------------------------------------------------------------------------------------------|
| `/login`       | `LoginPage`         | public     | WebFinger login + instance switcher                                                                  |
| `/register`    | `RegisterPage`      | public     | instance switcher (defaults to global domain) + CORS warning on a custom instance, then auto-logs in |
| `/`            | `GalleryPage`       | required   | the main three-pane workspace                                                                        |
| `/tags`        | `TagsPage`          | required   | placeholder (tag tree lives in the gallery panel)                                                    |
| `/tagging`     | `TaggingPage`       | required   | tagging-pipeline editor                                                                              |
| `/tagging/:id` | `ServiceEditorPage` | required   | single tagging-service editor                                                                        |
| `/shares`      | `SharesPage`        | required   | placeholder (share UI lives in the gallery panel)                                                    |
| `/settings`    | `SettingsPage`      | required   | profile + versioning mode + trash retention (reached via user menu)                                  |
| `/trash`       | `TrashPage`         | required   | soft-deleted photos grid with per-item / restore + purge countdown                                   |
| `/admin`       | `AdminPage`         | admin only | placeholder                                                                                          |
| `*`            | → `/`               | —          |                                                                                                      |

`ProtectedRoute` (`components/layout/ProtectedRoute.tsx`) gates auth and (with `adminOnly`) the admin role. Authenticated routes render inside
`AppShell` (unified `TopBar` + routed `<Outlet/>`). **There is no `/photos/:id`** — full-size viewing is the `Lightbox` carousel and details live in
the
right panel.

---

## 5. State management

### Zustand stores (`src/stores/`)

| Store          | Shape                                                                                                                                                                                                         | Persistence (`localStorage`) |
|----------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------|
| `auth.ts`      | `user, accessToken, refreshToken, backendUrl, instance` + setters/`clear`                                                                                                                                     | `archypix_auth`              |
| `ui.ts`        | `leftSidebarOpen, rightSidebarOpen, leftSidebarWidth, rightSidebarWidth, rowHeight, tagProvenance` + actions (`setLeftOpen/setRightOpen/setLeftWidth/setRightWidth`, clamped to `[SIDEBAR_MIN, SIDEBAR_MAX]`) | `archypix_ui`                |
| `theme.ts`     | `theme: 'dark' \| 'light'` (applies/removes `.light`); `initTheme()` at boot                                                                                                                                  | `archypix_theme`             |
| `selection.ts` | `selected: string[], anchor` — gallery multi-select (click / ⌘-toggle / shift-range)                                                                                                                          | none (session only)          |
| `upload.ts`    | `open, initialFiles, openDialog(files?), closeDialog` — upload dialog trigger shared by `TopBar` and `GalleryPage`                                                                                            | none (session only)          |

`hooks/usePersistentBool.ts` persists individual booleans (used for foldable detail-section collapse under `archypix_ui_section_<id>`).

### URL as view state (`hooks/useGalleryParams.ts`)

The gallery view lives entirely in the URL so it is shareable and back/forward-friendly. `useGalleryParams()` returns typed `params`, derived
`filters` (for `usePictures`), and `update(patch, { replace })`. Params (defaults are **omitted** from the URL):

| Param              | Meaning                                                                              |
|--------------------|--------------------------------------------------------------------------------------|
| `q`                | filename search (client-side filter — see §9)                                        |
| `tag`              | active tag filter (wire form)                                                        |
| `scope`            | `all` \| `owned` \| `shared`                                                         |
| `deleted`          | include trashed (`1`)                                                                |
| `sort`             | `ingested_at` \| `captured_at` \| `updated_at`                                       |
| `order`            | `asc` \| `desc`                                                                      |
| `after` / `before` | capture-date bounds (ISO)                                                            |
| `panel`            | active left tab: `tags` \| `incoming` \| `outgoing` \| `hierarchies`                 |
| `share`            | incoming share id to highlight (cross-link target)                                   |
| `hierarchy`        | active hierarchy id — center grid browses it (via `browse`) instead of the flat list |
| `hpath`            | directory path within the active hierarchy (slash-separated names, `''` = root)      |
| `hedit`            | hierarchy id whose config editor occupies the center view (overrides the grid)       |
| `view`             | open the Lightbox on this picture id (set by `PhotoGrid`)                            |

---

## 6. Data layer

One file per domain under `src/api/` (typed axios wrappers using `apiClient`), with matching hooks under `src/hooks/` (TanStack Query). Types live in
`src/lib/types.ts`; query keys are centralized in `src/lib/constants.ts` (`queryKeys`).

| Domain      | `api/*`                                                                                                                                                      | `hooks/*`                                                                                                                                     |
|-------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------|
| auth        | `auth.ts` — `login, logout, register, fetchMe`                                                                                                               | (imperative; not query-backed)                                                                                                                |
| pictures    | `pictures.ts` — `listPictures, getPicture, getPictureUrl, editPicture, overrideExif, trashPicture, restorePicture, getJob, beginUploadBatch, completeUpload` | `usePictures` (infinite, `thumbnail:'medium'`, page 50), `usePictureEdit.{useEditExif, useOverrideExif, useTrashMutations}`                   |
| tags        | `tags.ts` — `listAllTags, listPictureTags, listPictureTagsWithSources, batchEditTags`                                                                        | `useTags` — `useAllTags, usePictureTags, useBatchEditTags`                                                                                    |
| shares      | `shares.ts` — `list/accept/reject/revoke/createOutgoing`                                                                                                     | `useShares` — `useIncomingShares, useOutgoingShares, useShareMutations`; `useShareMappings`                                                   |
| tagging     | `tagging.ts` — service + rule/segment/mapping CRUD, `reorderServices`                                                                                        | `useTaggingServices` — `useTaggingServices, useTaggingService, useTaggingMutations`                                                           |
| hierarchies | `hierarchies.ts` — CRUD + `getHierarchyTree`, `browseHierarchy`, `getWebdav`/`regenerateWebdavToken`/`setWebdavUseRedirect`                                  | `useHierarchies` — `useHierarchies, useHierarchy, useHierarchyTree, useHierarchyBrowse, useHierarchyMutations, useWebdav, useWebdavMutations` |
| settings    | `settings.ts` — `getSettings, updateSettings` (versioning + `trash_retention_days`)`, updateProfile`                                                         | `useSettings` — `useSettings, useUpdateSettings, useUpdateProfile`                                                                            |

`apiErrorMessage(error)` (in `api/client.ts`) extracts a human string for toasts. `hooks/useDebouncedValue.ts` backs the search box.

**Tag paths** are dot-separated ltree **wire form** (`Photos.Travel.Alps`) on the wire and slash **display form** (`/Photos/Travel/Alps`) in the UI.
Convert via `lib/utils.ts` → `TagPath`: `toDisplay`, `toWire`, `segments`, `leaf`, `isProtected`. Share identities encode `@`→`_AT_`, `.`→`_DOT_`
within a label (e.g. `SharedToMe.alice_AT_ex_DOT_com.Photos`). `SharedToMe` is the reserved **protected** prefix.

---

## 7. Component map (`src/components/`)

**`layout/`** — `AppShell` (chrome; top bar + routed content + `StatusBar`; also owns the single `UploadDialog` instance), `TopBar` (single unified
bar:
brand + nav + sidebar toggles + gallery search/filters + **Upload button** + theme + user; gallery-only controls keyed on `pathname === '/'`. The
primary
nav collapses into the user dropdown on mobile (`< md`); **Settings** lives in the user dropdown, not the nav), `StatusBar` (thin footer: placeholder
storage gauge, tagging-services count, and — on the gallery — current view, selection count, and the thumbnail-size slider), `SidePanel` (resizable
workspace panel: inline + drag-handle on desktop, overlay drawer + backdrop on mobile; width persisted by the caller), `LeftPanel` (shadcn `Tabs`:
Tags /
Incoming / Outgoing / Hierarchies, synced to the `panel` URL param — chrome-less content, wrapped by `SidePanel`), `ProtectedRoute`,
`PagePlaceholder`.

**`photos/`** — `PhotoGrid` (justified flex grid + infinite scroll + selection + renders the Lightbox), `PhotoCard` (`flex-basis`/`flex-grow` from the
picture's **display** aspect ratio + `aspect-ratio` on the cell → uniform row height, no crop), `OrientedImage`/`OrientedContainImage` (render raw
thumbnails at their correct EXIF orientation — see §9), `Blurhash`, `FilterControls` (search + sort + filters dropdown — scope folded into Filters —
rendered inside `TopBar`), `Lightbox` (full-screen carousel driven by the `view` param; ←/→/Esc; `large` variant; header carries a trash/restore
action), `SelectionPanel`
(right panel; see §8), `PhotoCard` (also surfaces trash state — dimmed + a corner trash chip when `deleted_at` is set — and a **red** owner chip with
an alert icon when a received picture's `owner_deleted_at` is set), `UploadDialog` (batch upload with drag-and-drop, per-file progress, and initial
tag assignment — see §9).
**`photos/detail/`** — `Section` (compact foldable section, collapse persisted per id), `ExifInlineEditor` (presentational inline per-field EXIF
editor:
blue-dot dirty indicator, per-field reset on hover, save button in section header, exif_sync_status badge, unit-prefixed/suffixed inputs; every
editable
row — including the exposure num/den pair on one line — is **click-to-edit** (shows formatted text, swaps to inputs on click); orientation is **not**
a row —
it is driven by the preview's rotate overlays. **Received pictures are editable too** — each edit becomes a recipient-local override; an overridden
field shows an `OverwrittenBadge`), `OverwrittenBadge` (amber "overwritten" chip + tooltip explaining that the override is private and **not** visible
in WebDAV, which serves the owner's file directly; the ✕ drops the override so the owner's value flows back), `DateTimePickerPopover` (shadcn Calendar

+ time input, auto-applies on change → `NaiveDateTime` string
`YYYY-MM-DDTHH:MM:SS`, **no timezone**; Clear resets and closes the popover), `GpsPickerPopover` (interactive map + manual lat/lng/alt inputs +
"current location" + Clear). The draft state is owned by `hooks/useExifDraft.ts` (shared between the editor and the preview's rotate buttons; re-seeds
from server
  state on the picture's `id`+`updated_at` signature, exposes `set/setGps/reset/resetGps/rotate/save`, and — for received pictures — `owned`,
  `overriddenKeys`, and `removeOverride(...fields)`; it routes saves through `useEditExif` for owned pictures and `useOverrideExif` (DB-only, no job
  poll) for received ones). **Orientation is excluded from the manual
dirty/save flow** — `rotate` updates the draft for instant feedback and auto-commits the new orientation after a 700 ms debounce (
`set: { orientation }`),
so the rotate buttons never leave the EXIF section "dirty".

**Leaflet is loaded from CDN at runtime** (vanilla `leaflet` via a one-time injected `<script>`/`<link>`, no npm package) — the `react-leaflet`
wrapper
pulled a duplicate React copy under the project's mixed npm/pnpm `node_modules` and crashed with "Invalid hook call". Vanilla Leaflet has no React
dependency, so the map is driven imperatively. The loader + typed surface live in `lib/leaflet.ts`; the factored
`components/common/MapView` renders it in three modes — **point** (a draggable pin; EXIF GPS picking), **bbox** (a
rectangle with draggable corner + centre handles), **circle** (centre + radius handles) — reused by both `GpsPickerPopover`
(point) and the rules' `MapZonePopover` (bbox/circle). The **basemap is user-selectable** (`BASEMAPS` in `lib/leaflet.ts`:
Streets = CARTO Voyager default, Satellite = Esri World Imagery, OSM, Light, Dark — all free/no-key, choice persisted in
`stores/mapStyle.ts`). `MapView` also offers a **"center on my location"** control, an **enlarge** button (opens the map
in a large modal `Dialog`), and **favourite locations** — saved points (localStorage, `stores/favoriteLocations.ts`)
shown as star pins (click to centre the pin/rect/circle on them), saved via the ★ control and renamed inline (default
name = coordinates).

**`tags/`** — `TagTree` (recursive hierarchy from `useAllTags`; click sets the `tag` filter and **clears any active `hierarchy`/`hpath`** so the tag
filters the flat gallery rather than the current hierarchy directory; auto-expands ancestors of the active tag and scrolls it
into
view when it changes externally), `TagPicker` (autocomplete over existing tags + create-new; `allowProtected` prop — see §9; optional `trigger` prop
to
render a custom trigger, e.g. the small **+** button in the details-panel Tags section header).

**`tagging/`** — `TaggingPage` (header has a **Force run** button — `POST /pictures/pipeline/wake` — for debugging) composes `SharedMappingSection`
(shared-tag-mapping services in a **collapsed-by-default accordion, always first**)
then
`PipelineList` (rule + segmentation services, **@dnd-kit reorder that never includes shared_tag_mapping ids**) of `ServiceCard`s.
`RequiresExcludesEditor`
(gates as a **local draft committed on Save**), `RuleEditor`, `SegmentEditor`, `MappingEditor`, `DeleteServiceDialog` (promote-vs-remove),
`NewServiceMenu`. `RuleEditor` builds the structured predicate tree (feature 13) with `PredicateBuilder` — a nested
AND/OR/NOT block composer where **a single root @dnd-kit `DndContext` lets you drag a block between levels** (out of a
group, into a sibling group) as well as reorder within one; field-condition leaves pick a field via a **grouped,
searchable `FieldPicker`** (Dates / Camera / Location / File / Ownership) + a type-aware operator/value (numbers use the
`NumberInput` stepper component), date conditions use the shared `DateRangePicker` (datetime), and GPS leaves open
`MapZonePopover` (rectangle/circle on a real map). Existing rules are **editable inline** (hydrated via
`lib/predicate.ts:deserialize`) and **drag-reorderable** (persisted via `POST …/rules/reorder`). Predicate model +
serialize/deserialize/describe + tree-move helpers live in `lib/predicate.ts`. **Every service (rule / segmentation /
shared-mapping) is inline-renameable** via `ServiceNameEditor` (the card, the editor-page header, and the shared-mapping
accordion all use it). `SegmentEditor` uses the `DateRangePicker` (datetime mode) emitting **NaiveDateTime** strings (no
timezone, as the backend requires).

**`shares/`** — both lists split shares into **Closed (revoked + tombstoned, collapsed by default) / Pending / Active** foldable `Section`s (so the
section already conveys status — there is **no** inline status badge on a card) and surface each share's details through `ShareInfoPopover` (an `Info`
trigger button — hover on desktop, tap on touch — anchored to the **right**, towards the pictures pane). The popover is detail-rich: `name`, the
`ShareStatusBadge`, ShareBack-allowed / future-additions rendered as compact on/off **`FlagChip`s** (consistent emerald-on / muted-off styling, not a
right-aligned Yes/No), the shared tag, created date, ShareBack provenance ("which share this answers"), and — by side — the last-announcement
timestamp
(incoming), `last_error_at` / `next_retry_at` (outgoing, while errored/recovering), and the close date for revoked/rejected shares, then the `message`
("No message" when null), plus an optional `footer` slot. `IncomingSharesList` keeps flat rows (accept / reject(confirm) / view-photos; single
local-tag
mapping per share via `useShareMappings`) and, when the sender allows it, a **Share back** button in the popover footer that opens a controlled
`CreateShareDialog` pre-targeted at the sender and pre-filled with that share's mapped local tag (if a `SharedTagMappingService` mapping exists; still
editable). `OutgoingSharesList` groups by tag via a factorized
`GroupedShareRow` reused across all three sections — the group header shows the most common `name` with a "(and N others)" suffix when names differ
(`summarizeNames`), the per-recipient details living in the popover, plus confirm-revoke. Both lists cross-reference the other direction (incoming ↔
outgoing) to resolve the ShareBack provenance label. `CreateShareDialog` creates **one share per recipient**: an optional **Share back of** combobox
(lists the user's live incoming shares; selecting one marks the share a ShareBack — sends `shareback_of` = that incoming share's `outgoing_share_id`
and
locks the recipient to its sender), common `name` (≤ 64) / `message` (≤ 1000) / tag / ShareBack / future inputs and a recipient list of grouped
`@username:instance` fields (typing `:` advances focus to the instance sub-field, a button adds rows); on submit it fires one request per recipient
sequentially with per-recipient progress icons (mirrors `UploadDialog`). It supports a controlled `open`/`onOpenChange` (with `showTrigger=false`) and
an
`initialTag` prop so the ShareBack button can drive it pre-filled. `ShareStatusBadge` maps status → coloured pill (used inside the popover).

**`hierarchies/`** — `HierarchyPanel` (Hierarchies left tab: list of hierarchies + **New**, or, when one is active, a back header with a **WebDAV**
(HardDrive) and edit button over the directory
tree), `WebdavDialog` (mount-info popup: copyable mount URL, username (`@user`), and token — token hidden by default with show/copy buttons — a
**Regenerate token** button, and a `use_redirect` toggle; the token is only minted on open via `useWebdav`, since the GET endpoint mints on first
access),
`HierarchyDirTree` (lazy recursive directory tree from `tree` — each row fetches its own children on expand; clicking a folder drives the
center
grid via the `hierarchy`/`hpath` params; shows per-dir `picture_count` and a lock on read-only dirs), `CreateHierarchyDialog` (name → create empty →
open editor). The **central-view editor** (shown in the gallery center when `hedit` is set, replacing the grid): `HierarchyEditor` (header with
name/enabled/Save/Reset/Delete + a small **Braces** JSON-debug button; hierarchy-level settings — naming, safeDeleteMode, write-back master switch —
then the node tree; edits a local draft committed on Save via `PATCH`, re-validated server-side), `NodeEditor` (mutually-recursive `NodeListEditor` +
per-node card; add/remove/reorder of `mirror`/`query`/`static` nodes with their kind-specific fields and an Advanced disclosure for per-node
naming/safeDeleteMode), `WriteBackEditor` (query-node write-back op-lists with a "suggest from predicate" helper — forward-looking, exercised by
WebDAV), `TagListField` (chips + `TagPicker`, reused for include/exclude/collapsed), `JsonConfigDialog` (raw `config` textarea; applies to the draft).

**`common/`** — `ConfirmDialog` (AlertDialog wrapper gating sensitive actions), `MapView` (shared imperative Leaflet map;
point/bbox/circle modes), `DateRangePicker` (calendar range on the shadcn `Calendar`, **week starts Monday**; `date` mode
emits `YYYY-MM-DD` bounds, `datetime` mode emits NaiveDateTime bounds with **optional times** — default first-day 00:00:00 /
last-day 23:59:59).

---

## 8. The gallery workspace

`GalleryPage` is a three-pane layout under the unified `TopBar`; each side panel is wrapped in `SidePanel` (resizable on desktop, overlay drawer on
mobile) and shown only when its `ui` store toggle is on:

- **Left** (`LeftPanel`): tabbed Tags tree / Incoming shares / Outgoing shares / Hierarchies. The **Hierarchies** tab (`HierarchyPanel`) lists the
  user's hierarchies and, once one is picked, shows its navigable directory tree.
- **Center** (`PhotoGrid`): the justified grid; double-click opens the `Lightbox`; click an already-selected photo deselects it. When a `hierarchy`
  is active it browses that directory (via `useHierarchyBrowse`) with a clickable path breadcrumb instead of the flat picture list; when `hedit` is
  set
  the `HierarchyEditor` takes over the center (the right selection panel is suppressed while editing).
- **Right** (`SelectionPanel`): only mounted when a selection exists (returns `null` otherwise — its `SidePanel` wrapper additionally honours the
  `rightSidebarOpen` toggle). For a single selection: borderless thumbnail (click opens lightbox; received pictures get an `@owner:instance` label
  overlaid on the preview — **red with a tooltip when the owner has trashed the picture**; rotate overlays now also apply to received pictures, as an
  orientation override), filename + size/dimensions/mime inline, ingested/updated timestamps (formatted in the local timezone via `formatDateTime`),
  an **owner-deletion grace banner** (received, when `owner_deleted_at` is set — "disappears on *X*"), a **local-trash banner** (when the picture is
  in
  the holder's trash, with the owned purge date), and a **Move to trash** (ConfirmDialog) / **Restore** action,
  then foldable sections — **Tags** (chips; **+** add button and provenance toggle in the section header; provenance mode renders each path as a chip
  with colour-coded per-source mini-tags), **Shared with you** (sender handle + shared subpath, not the raw `SharedToMe.*` path), **Shared by you**,
  **EXIF** (inline-editable — owned pictures write through to the file, received pictures get recipient-local overrides; the badge flips to **modified
  **
  on unsaved changes, or **overridden** when a received picture has sticky overrides), **Versions**. Clicking
  a
  tag filters by it and reveals it in the Tags tree (opens the left panel + Tags tab + expands/scrolls the tree). For multi-selection: batch tag-add +
  batch trash/restore (one request per picture — there is no batch trash endpoint).

---

## 9. Key behaviours & gotchas

- **Thumbnails:** `usePictures` passes `thumbnail:'medium'` so list items carry presigned URLs — no per-card round-trip. Presigned URLs are cached in
  Query (`['pictures','url',id,variant]`, ~10 min `staleTime`); the Lightbox uses `large`.
- **Protected tags:** `TagPicker` hides `SharedToMe.*` unless `allowProtected` is set. It is **off** for manual tagging (SelectionPanel) and
  share-mappings; **on** only for `CreateShareDialog` (sharing) and `RequiresExcludesEditor` (service gates). Protected tags can never be *created*.
- **Tag removal is manual-only:** `batchEditTags` `remove_tags` only drops `manual` rows. In the provenance table the ✕ appears only on tags with a
  manual source; pipeline/share tags reappear after re-evaluation.
- **Pipeline is async:** tagging-service mutations invalidate `['tagging']`/`['tags']`/`['pictures']`, but the backend re-evaluates tags in the
  background — assignments converge after a short delay, not synchronously. Service *state* (enabled/gates) does update immediately on refetch; the
  pipeline list reads service objects fresh from props (keeps only drag order locally) to avoid stale toggles.
- **EXIF editing:** owned pictures (`picture.owner_username == null`) edit through `useEditExif`, which POSTs the diff (`set`/`clear`) then polls
  `getJob` (1/2/4/8/15 s) while `exif_sync_status === 'pending'`. **Received pictures** edit through `useOverrideExif` →
  `POST /pictures/{id}/exif/override`
  (DB-only, no job poll): `set` claims a sticky per-field override, `clear` (via `removeOverride`) drops it so the owner's value flows through again.
  Overridden fields are derived from `picture.local_exif_overrides` (a sparse `FullExif`, snake-case keys) and tagged with `OverwrittenBadge`. The
  override never touches the owner's file, so it is invisible over WebDAV — that caveat is the badge's tooltip.
- **Trash & restore:** `useTrashMutations` (`trash`/`restore`) POSTs `/pictures/{id}/trash` | `/restore` and invalidates `['pictures']` + the detail.
  Owned trash is purged after `trash_retention_days` (the `/trash` page derives the purge date as `deleted_at + retention` since the list item carries
  no `owner_purge_at` for owned rows); received trash is local-only. The gallery shows trashed items only with Filters → **Include trashed**; the
  `/trash` page fetches `include_deleted` and client-filters to `deleted_at != null`.
- **Orientation rendering:** thumbnails/originals are raw pixels (EXIF orientation not baked in). `OrientedImage` rotates at display time using
  `orientedCoverStyle` (absolute positioning + CSS transform; sets `max-w-none` to escape Tailwind's `img { max-width: 100% }` which otherwise
  collapses 90°/270° images). `OrientedContainImage` fits a rotated image into a variable-aspect container. The sidebar preview uses the live draft
  orientation for instant feedback on rotate clicks.
- **Single mapping per share:** `useShareMappings.addMapping` deletes any existing mapping first; the tagging `MappingEditor` hides already-mapped
  shares.
- **Cross-links:** right-panel tag → sets the `tag` filter; a provenance source badge → `/tagging/:source_id` (or `panel=incoming` + `share` highlight
  for an `incoming_share` source); a "Shared with you" tag → `tag` filter + `panel=incoming` + highlights the matching card in `IncomingSharesList`.
- **Search:** the API has **no free-text search**; `q` is a client-side filename filter over already-loaded items (it is still kept in the URL for
  future server-side search). The capture-date range filter has a reserved slot in the Filters menu but is not built yet.
- **Sensitive actions** (revoke / reject / delete) are gated by `ConfirmDialog`.
- **Upload flow:** `UploadDialog` (rendered once in `AppShell`) is triggered via `useUploadStore`. The `TopBar` Upload button and the `GalleryPage`
  full-page drag zone both call `openDialog(files?)`. The dialog batch-presigns all files (`POST /uploads/batch`), uploads to S3 in parallel (max 4
  concurrent, via `XMLHttpRequest` for per-file progress), and calls `POST /uploads/{id}/complete` immediately per file as its S3 PUT finishes — not
  after all files. Each file's SHA-256 is computed in parallel with its S3 PUT (`crypto.subtle.digest`, lowercase hex — the same digest the worker
  produces) and sent as `file_hash`. `initial_tags` set in the dialog are passed on the complete body and assigned atomically server-side. The backend
  wakes the pipeline through its debounced window, so per-file completions coalesce into a single run with no client-side defer/wake bookkeeping.
  Gallery and tags queries are invalidated on the first success and again when all uploads settle.

---

## 10. Conventions for working on the frontend

- **Adding an API call:** add the typed wrapper to the right `api/*.ts` (use `apiClient`, `import type` for types), a hook in `hooks/`, and a key in
  `queryKeys` (`lib/constants.ts`). Mutations invalidate the relevant key prefixes on success.
- **Errors:** surface with `toast.error(apiErrorMessage(e))` (sonner).
- **Tags on the wire are wire form**; convert at the UI boundary with `TagPath`.
- **View state belongs in the URL** (`useGalleryParams`); UI preferences belong in the `ui`/`theme` stores.
- **Strict TS:** type-only imports must use `import type`; avoid `any` (prefer `unknown`/`Record<string, unknown>`). `npm run build` (= `tsc -b` +
  `vite build`) must stay green.
- shadcn primitives are editable project files under `components/ui/`; custom domain components live in their domain folder.
- **Number inputs:** use `components/ui/number-input.tsx` (`NumberInput`) — native spin arrows are stripped in `index.css`; this component adds styled
  chevron steppers. Steppers auto-hide when `step="any"` (free-form decimals like GPS lat/lng).
