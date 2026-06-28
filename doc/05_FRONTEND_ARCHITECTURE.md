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
| Media player  | `@vidstack/react` (default skin) — inline video/audio playback (lazy-loaded chunk)      |

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
| `/tagging`     | `TaggingPage`       | required   | tagging-services editor                                                                              |
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

| Store          | Shape                                                                                                                                                                                                                                                                                                                                                                       | Persistence (`localStorage`) |
|----------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------|
| `auth.ts`      | `user, accessToken, refreshToken, backendUrl, instance` + setters/`clear`                                                                                                                                                                                                                                                                                                   | `archypix_auth`              |
| `ui.ts`        | `leftSidebarOpen, rightSidebarOpen, leftSidebarWidth, rightSidebarWidth, rowHeight, tagProvenance` + actions (`setLeftOpen/setRightOpen/setLeftWidth/setRightWidth`, clamped to `[SIDEBAR_MIN, SIDEBAR_MAX]`)                                                                                                                                                               | `archypix_ui`                |
| `theme.ts`     | `theme: 'dark' \| 'light'` (applies/removes `.light`); `initTheme()` at boot                                                                                                                                                                                                                                                                                                | `archypix_theme`             |
| `selection.ts` | the feature-14 **selection descriptor** `query: PictureFilter \| null, includeIds, excludeIds, anchor, multiSelect` (explicit mode = `query null`; select-all = an adopted view `query` + `excludeIds`; helpers `isMemberSelected`/`toApiSelection`/`hasSelection`/`isSingleSelection`; click / ⌘-toggle / shift-range / ⌘A; `multiSelect` = touch long-press mode, see §9) | none (session only)          |
| `upload.ts`    | `open, initialFiles, openDialog(files?), closeDialog` — upload dialog trigger shared by `TopBar` and `GalleryPage`                                                                                                                                                                                                                                                          | none (session only)          |

`hooks/usePersistentBool.ts` persists individual booleans (used for foldable detail-section collapse under `archypix_ui_section_<id>`).

### URL as view state (`hooks/useGalleryParams.ts`)

The gallery view lives entirely in the URL so it is shareable and back/forward-friendly. `useGalleryParams()` returns typed `params`, derived
`filters` (for `usePictures`), and `update(patch, { replace })`. Params (defaults are **omitted** from the URL):

| Param              | Meaning                                                                              |
|--------------------|--------------------------------------------------------------------------------------|
| `tag`              | active (primary) tag filter (wire form) — set by a plain tag click                   |
| `inc` / `exc` / `exa` | extra compound-filter tag sets (comma wire paths): include / exclude / exact (strict) — built from the tag sidebar `…` menu |
| `scope`            | `all` \| `owned` \| `shared`                                                         |
| `deleted`          | include trashed (`1`)                                                                |
| `sort`             | `captured_at` (default) \| `ingested_at` \| `updated_at` \| `file_size` \| `filename` |
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

| Domain      | `api/*`                                                                                                                                                                                                                                                                                            | `hooks/*`                                                                                                                                                                                                                                                                 |
|-------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| auth        | `auth.ts` — `login, logout, register, fetchMe`                                                                                                                                                                                                                                                     | (imperative; not query-backed)                                                                                                                                                                                                                                            |
| pictures    | `pictures.ts` — `listPictures, getPicture, getPictureUrl, editPicture, overrideExif, trashPicture, restorePicture, getJob, beginUploadBatch, completeUpload`; **batch (feature 14)** `aggregatePictures, batchEditExif, batchTrash, batchRestore` (all over a `PictureSelection`, `dry_run`-aware) | `usePictures` (infinite, `thumbnail:'medium'`, page 50), `usePictureEdit.{useEditExif, useOverrideExif, useTrashMutations}`; `useAggregate(selection, sections)` (debounced, lazy sections) + `useSelectionCount`; `useBatch.useBatchMutations` (trash/restore/tags/exif) |
| tags        | `tags.ts` — `listAllTags, listPictureTags, listPictureTagsWithSources, batchEditTags` (selection + `dry_run`-aware)                                                                                                                                                                                | `useTags` — `useAllTags, usePictureTags, useBatchEditTags`                                                                                                                                                                                                                |
| shares      | `shares.ts` — `list/accept/reject/revoke/createOutgoing`                                                                                                                                                                                                                                           | `useShares` — `useIncomingShares, useOutgoingShares, useShareMutations`; `useShareMappings`                                                                                                                                                                               |
| tagging     | `tagging.ts` — service + rule/segment/mapping CRUD, `reorderServices`                                                                                                                                                                                                                              | `useTaggingServices` — `useTaggingServices, useTaggingService, useTaggingMutations`                                                                                                                                                                                       |
| hierarchies | `hierarchies.ts` — CRUD + `getHierarchyTree`, `browseHierarchy`, `getWebdav`/`regenerateWebdavToken`/`setWebdavUseRedirect`                                                                                                                                                                        | `useHierarchies` — `useHierarchies, useHierarchy, useHierarchyTree, useHierarchyBrowse, useHierarchyMutations, useWebdav, useWebdavMutations`                                                                                                                             |
| settings    | `settings.ts` — `getSettings, updateSettings` (versioning + `trash_retention_days`)`, updateProfile`                                                                                                                                                                                               | `useSettings` — `useSettings, useUpdateSettings, useUpdateProfile`                                                                                                                                                                                                        |

`apiErrorMessage(error)` (in `api/client.ts`) extracts a human string for toasts. `hooks/useDebouncedValue.ts` debounces the batch `/aggregate`
descriptor and EXIF dry-run previews.

**Tag paths** are dot-separated ltree **wire form** (`Photos.Travel.Alps`) on the wire and slash **display form** (`/Photos/Travel/Alps`) in the UI.
Convert via `lib/utils.ts` → `TagPath`: `toDisplay`, `toWire`, `segments`, `leaf`, `isProtected`. Share identities encode `@`→`_AT_`, `.`→`_DOT_`
within a label (e.g. `SharedToMe.alice_AT_ex_DOT_com.Photos`). `SharedToMe` is the reserved **protected** prefix.

---

## 7. Component map (`src/components/`)

**`layout/`** — `AppShell` (chrome; top bar + routed content + `StatusBar`; also owns the single `UploadDialog` instance), `TopBar` (single unified
bar:
brand + nav + sidebar toggles + gallery search/filters + **Upload button** + theme + user; gallery-only controls keyed on `pathname === '/'`. The
primary
nav collapses into the user dropdown on mobile (`< md`); **Settings** lives in the user dropdown, not the nav. On mobile the **theme toggle** also
moves into the user dropdown and the **right-panel (details) toggle is dropped** — a single tap opens the details drawer and multi-select uses the
floating bar (§8/§9) — so the bar stays minimal), `StatusBar` (thin footer: placeholder
storage gauge, tagging-services count, and — on the gallery — current view, selection count, and the thumbnail-size slider), `SidePanel` (resizable
workspace panel: inline + drag-handle on desktop, overlay drawer + backdrop on mobile; width persisted by the caller), `LeftPanel` (shadcn `Tabs`:
Tags /
Incoming / Outgoing / Hierarchies, synced to the `panel` URL param — chrome-less content, wrapped by `SidePanel`), `ProtectedRoute`,
`PagePlaceholder`.

**`photos/`** — `PhotoGrid` (justified flex grid + infinite scroll + selection + renders the Lightbox), `PhotoCard` (`flex-basis`/`flex-grow` from the
picture's **display** aspect ratio + `aspect-ratio` on the cell → uniform row height, no crop; sits on a **`.bg-checkerboard`** backdrop (see §9) and
**fades its blurhash out once the thumbnail loads** so transparent PNG areas read as transparent, not blurry), `OrientedImage`/`OrientedContainImage`
(render raw thumbnails at their correct EXIF orientation — see §9; `OrientedContainImage`'s sized box also carries `.bg-checkerboard`), `Blurhash`
(loading placeholder only — faded out on load), `FilterControls` (**Sort** + **Filters** dropdowns; rendered inside `TopBar`.
Sort offers Date taken / added / modified, File size, Name — default **Date taken** (`captured_at`); Filters folds in scope, Include-trashed,
and a **capture-date range** using the shared `DateRangePicker` calendar — no tag chips here, those live in the centre `TagFilterBar`),
`TagFilterBar` (`components/tags/`, a breadcrumb-style bar atop the flat grid showing the active include / `=`-exact / `⦸`-exclude tags as chips,
each with a switch-include↔exact control + remove, plus Clear), `Lightbox` (full-screen carousel driven by the `view` param;
←/→/Esc, plus **Delete/⌘+Backspace trashes the picture in view immediately, no confirm dialog** — both that shortcut
and the header's trash button (via `ConfirmDialog`) then **advance to the next picture (or previous if it was last)
instead of closing**, only closing when no picture remains;
always the `large` variant; **portaled to `document.body`** so it paints above the mobile sidebar drawer while the trash confirm dialog still stacks
on
top; **click the backdrop outside the image to close** — and closing **selects the viewed picture** (opening the right drawer on mobile) so the user
lands on its specs; header carries **download-original**, rotate-left/right (auto-committing orientation via `useExifDraft`, same as the
sidebar), and a trash (`ConfirmDialog`)/restore action), `SelectionPanel`
(right panel; see §8), `PhotoCard` (also surfaces trash state — dimmed + a corner trash chip when `deleted_at` is set — and a **red** owner chip with
an alert icon when a received picture's `owner_deleted_at` is set), `UploadDialog` (batch upload with drag-and-drop, per-file progress, and initial
tag assignment — see §9), `MediaPlayer` (Vidstack wrapper — picks the default video/audio layout from the picture's mime; used by the
Lightbox and the details panel — see §9).
**`photos/batch/`** (feature 14 multi-select) — `SelectionActionBar` (floating bar on **desktop and mobile** whenever
more than one picture is selected; the full-width container is `pointer-events-none` so only the pill catches clicks:
resolved count via `useSelectionCount`, **Select-all** (adopts the view's `PictureFilter`; `⌘/Ctrl+A` does the same),
**Invert** (`swap(include, exclude)` — adopts the view query when none was set), **Clear**, and **Batch actions** which
surfaces the right panel — hidden on desktop when the panel is already docked open), `MultiSelectionPanel` (the right
panel for a multi-selection: header count + Clear, then a **Trash / Restore** button row (same line, wrapping; each
disabled when it would change nothing) and foldable **Summary** / **Tags** / **Info** / **EXIF** sections fed by
`useAggregate` with per-section lazy fetch; an EXIF-sync convergence line ticks while a deferred batch edit drains,
§6.3; every section shows "No …" when the selection resolves to 0). **Tags** are shown one-per-line with `tag_provenance`
always on, wrapping the per-source mini-tags (carrying each source's cardinality) under the path chip; the chip is a
tristate (solid on-all vs dashed on-some `count/total`) with a shadcn **tooltip** of the full path, **+** adds via
`TagPicker`, ✕ (highlighted on hover) removes only where `manual_count > 0`. `BatchMetadataSection` ("Info") holds the
read-only file aggregates (size / dimensions / type / added / edited). `BatchExifSection` is **inline-editable** and
mirrors the single-picture EXIF editor's field order (captured, GPS, camera brand/model, focal, aperture, ISO, exposure —
num/den **merged** into one rational row; orientation hidden); field names are green `FieldLabel` chips, values show the
common value or amber **Mixed**, a stats sub-row carries the range · avg · `n/total set` and (for strings) the first 10
distinct values with an **(i)** popover for the rest. Editable fields are click-to-edit into a draft (empty ⇒ clear); a
**Save** in the section header opens `BatchConfirmDialog` which computes the `dry_run` **only on open** (with a
local/suggest mode toggle re-running it when received pictures are present); the GPS aggregate renders its bbox on a
read-only `MapView`. `BatchConfirmDialog` is the mandatory confirmation gate: runs the endpoint's `dry_run` on open
(re-running when `dryRunKey` changes) before enabling Confirm; trigger-based or programmatically-controlled.
**`photos/detail/`** — `Section` (compact foldable section, collapse persisted per id — optionally **controlled** via `open`/`onOpenChange` for lazy
fetching), `ExifInlineEditor` (presentational inline per-field EXIF
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
"current location"; a real **close** (✕) button sits in the header corner and **Clear** lives at the bottom — so the corner button isn't mistaken
for a clear). The draft state is owned by `hooks/useExifDraft.ts` (shared between the editor and the preview's rotate buttons; re-seeds
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

**`tags/`** — `TagTree` (recursive hierarchy from `useAllTags`; a plain click sets the `tag` filter as the sole include and **clears any compound
filter + active `hierarchy`/`hpath`**; auto-expands ancestors of the active tag and scrolls it into view when it changes externally. Each row has a
**`…` menu** with toggle actions **Include / Include exactly / Exclude** (writing the `inc`/`exa`/`exc` params), and **⌘/Ctrl-click** quick-toggles a
tag in the include set to build "X and Y" fast; "Include exactly" is the strict/no-descendant mode (backend `exact`). The tree only **highlights**
rows by state — emerald (included/exact, `=` icon) or struck-through red (excluded, `⦸` icon); the active filter itself is surfaced in the centre
`TagFilterBar` breadcrumb (not in the tree)), `TagPicker` (autocomplete over existing tags + create-new; `allowProtected` prop — see §9; optional `trigger` prop
to
render a custom trigger, e.g. the small **+** button in the details-panel Tags section header. Each list row carries a **›** button (and **Tab**
autocompletes the highlighted tag) that fills the field with `<tag>/` so the user can append a child without retyping — e.g. autocomplete `/Event`
then type `Birthday`. Input is **sanitized live**: accents stripped, spaces/`-` → `_`, `.`/`\` → `/` with an **amber** note of what was replaced, and a
**red** warning for a reserved `SharedToMe` prefix or any still-invalid character).

**`tagging/`** — `TaggingPage` (titled **"Tagging services"**; header has a **Force run** button — `POST /pictures/pipeline/wake` — for debugging)
composes `SharedMappingSection`
(shared-tag-mapping services in a **collapsed-by-default accordion, always first**)
then
`PipelineList` (rule + segmentation services, **@dnd-kit reorder that never includes shared_tag_mapping ids**) of `ServiceCard`s (a compact row —
type badge, name, item count, enabled switch, a prominent **Edit** button and delete; the requires/excludes **gates are not on the card**, only on the
editor page, so the list stays short with many services).
`RequiresExcludesEditor`
(gates as a **local draft committed on Save**, on the editor page), `RuleEditor`, `SegmentEditor`, `MappingEditor`, `DeleteServiceDialog` (promote-vs-remove),
`NewServiceMenu`. `RuleEditor` builds the structured predicate tree (feature 13) with `PredicateBuilder` — a nested
AND/OR/NOT block composer where **a single root @dnd-kit `DndContext` lets you drag a block between levels** (out of a
group, into a sibling group) as well as reorder within one; field-condition leaves pick a field via a **grouped,
searchable `FieldPicker`** (Dates / Camera / Location / File / Ownership) + a type-aware operator/value (numbers use the
`NumberInput` stepper component; **is set** / **is not set** are two distinct operators with no value field — both serialize to the `is_present`
leaf; string comparisons carry an **ignore case** checkbox → the `ignore_case` sibling flag, replacing the old `eq_ic` operator), date conditions use the shared `DateRangePicker` (datetime), and GPS leaves open
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
point/bbox/circle modes — in every mode **clicking the map moves the pin / rect / circle centre** there; custom app-styled zoom ±, **re-center on the selection**, my-location, save-favourite and enlarge
controls + a basemap switcher; `isolate`d so its z-indexes can't paint over the selection bar/dialogs. `interactive={false}`
(batch GPS aggregate) still pans/zooms/switches style and shows favourite stars, but drops the draggable shape handles, the
save-favourite button and the favourites bar), `DateRangePicker` (calendar range on the shadcn `Calendar`, **week starts Monday**; `date` mode
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
- **Right** (`SelectionPanel`): **presence follows only the `rightSidebarOpen` toggle** (default open), decoupled from the selection so the grid never
  shifts; with nothing selected it shows an unobtrusive **empty placeholder** ("No photo selected"). Selecting a photo no longer force-opens the panel
  on desktop (on mobile a tap still opens the right drawer). For a single selection: borderless thumbnail (click opens lightbox; received pictures get an `@owner:instance` label
  overlaid on the preview — **red with a tooltip when the owner has trashed the picture**; rotate overlays now also apply to received pictures, as an
  orientation override), filename + size/dimensions/mime inline, ingested/updated timestamps (formatted in the local timezone via `formatDateTime`),
  an **owner-deletion grace banner** (received, when `owner_deleted_at` is set — "disappears on *X*"), a **local-trash banner** (when the picture is
  in
  the holder's trash, with the owned purge date), a **download-original** button, a **Copy to my library** ("rescue", feature 11) action for received
  pictures (also a prominent button inside the owner-deletion grace banner, and a copy-of provenance line when an owned picture is a copy), + a **Move
  to trash** (ConfirmDialog) / **Restore** action,
  then foldable sections — **Tags** (chips; **+** add button and provenance toggle in the section header; provenance mode renders each path as a chip
  with colour-coded per-source mini-tags), **Shared with you** (sender handle + shared subpath, not the raw `SharedToMe.*` path), **Shared by you**,
  **EXIF** (inline-editable — owned pictures write through to the file, received pictures get recipient-local overrides; the badge flips to **modified
  **
  on unsaved changes, or **overridden** when a received picture has sticky overrides), **Versions**, and **Copies**
  (`CopiesSection`, feature 11 — lazily lists the picture's content-dedup group: each physical copy's state
  shown/in-trash/duplicate/rejected, owner, last-edit, a same-image-vs-EXIF-only-vs-different-content diff, and a
  "Keep this" control to choose the kept survivor). Clicking
  a
  tag filters by it and reveals it in the Tags tree (opens the left panel + Tags tab + expands/scrolls the tree). For multi-selection the panel
  renders
  `MultiSelectionPanel` (feature 14, §7 `photos/batch/`): Summary / tristate Tags / type-aware EXIF aggregates from
  `POST /pictures/aggregate`, plus selection-based batch tags / EXIF / trash / restore — each gated by a `dry_run` confirmation popup.

---

## 9. Key behaviours & gotchas

- **No-thumbnail fallback:** the backend returns `thumbnail_url` / the `/url` `url` as **`null`** for a thumbnail variant when the picture has no
  generated thumbnail (pending, or a non-thumbnailable format like a PDF, or audio) — never a fake/404 URL. The client renders a **`FileTypeIcon`**
  (`components/photos/`, a lucide icon picked from MIME or filename extension) instead: in the grid (`PhotoCard`), the sidebar preview, and the
  `Lightbox` ("No preview available — use Download"). `getPictureUrl` returns `url: string | null` accordingly (`original` is always a URL). **Videos
  do have thumbnails** (a worker frame-grab), so they render a real thumbnail with a play badge, not the icon fallback.
- **Video/audio playback (Tier 1):** `video/*` and `audio/*` pictures (detected via `isPlayableMedia(mime)`, `lib/utils.ts`) play the **`original`**
  file straight from S3 through the `MediaPlayer` (Vidstack) — progressive HTTP-Range playback, **no transcode/streaming infra**. The **Lightbox**
  autoplays: video fills the viewer like an image (`LightboxVideo` measures the area and sizes the player to the largest box of the video's aspect
  ratio that fits — contain); audio is a centred player bar. The **details panel** plays audio inline; for video it shows the frame-grab thumbnail as
  a
  poster that opens the (autoplaying) Lightbox — the cramped panel isn't for watching. **Grid cards never mount a player.** The play badge is the
  shared
  `PlayBadge` (same icon in the grid and the details poster) and is shown **only over a real frame thumbnail** — a video with no thumbnail (or audio)
  falls back to a bare `FileTypeIcon` with no badge. The
  S3 object's `Content-Type` drives decoding, so only browser-playable codecs work (MP4/H.264, WebM, MP3/AAC/Ogg); `.mov`/HEVC, `.avi`, `.mkv` won't
  decode — Download-original is the fallback until a transcode worker (Tier 2) lands. Cross-instance media follows the same direct-presign path as
  images, so the owning backend's S3 CORS must allow `GET`/`Range`.
- **Thumbnails & adaptive sizing:** list items carry a presigned thumbnail URL (no per-card round-trip). The requested variant is **sized to how the
  picture is displayed** via `variantForSize(cssPx)` (`lib/utils.ts` — maps a **logical** display height to the worker's variant heights small=100 /
  medium=500 / large=1000, with thresholds `≤150 → small`, `≤350 → medium`, else `large`; **no `devicePixelRatio` multiplier** — slight upscaling on
  hi-DPI is an accepted trade-off for lighter payloads). The **grid** picks from the zoom (`rowHeight`, so the minimum zoom yields `small`) and
  threads
  it into `usePictures`/`useHierarchyBrowse` (variant is in the query key, so crossing a threshold refetches; `placeholderData: keepPreviousData`
  keeps
  the grid visible meanwhile); the **sidebar** preview picks from its capped display height (`PREVIEW_MAX_HEIGHT = 208` → `medium`, not the sidebar
  width); the **Lightbox** always uses `large`. Presigned URLs are cached in Query (`['pictures','url',id,variant]`, ~10 min `staleTime`).
  `downloadOriginal(id, filename)` (`api/pictures.ts`) fetches the `original` and saves it under the original filename via a blob + `download`
  attribute
  (this only sets the name when the cross-origin fetch succeeds; it falls back to opening the presigned URL, which downloads under the S3 key — the
  only
  way to force the filename in that case is the backend adding `response-content-disposition` at presign time). Wired to the download buttons in the
  Lightbox header and the sidebar.
- **Protected tags:** `TagPicker` hides `SharedToMe.*` unless `allowProtected` is set. It is **off** for manual tagging (SelectionPanel) and
  share-mappings; **on** only for `CreateShareDialog` (sharing) and `RequiresExcludesEditor` (service gates). Protected tags can never be *created*.
- **Tag removal is manual-only:** `batchEditTags` `remove_tags` only drops `manual` rows. In the provenance table the ✕ appears only on tags with a
  manual source; pipeline/share tags reappear after re-evaluation.
- **Pipeline is async:** tagging-service mutations invalidate `['tagging']`/`['tags']`/`['pictures']` immediately **and again ~1.5 s later** (the
  backend re-evaluates tags in the background, so the converged tags/pictures show up without a manual refresh). Service *state* (enabled/gates) does
  update immediately on refetch; the pipeline list reads service objects fresh from props (keeps only drag order locally) to avoid stale toggles.
  Navigating the **`TagTree`** (picking or expanding/collapsing a tag) also invalidates `['tags']` so the tree keeps up with background tag changes.
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
  collapses 90°/270° images). `OrientedContainImage` fits a rotated image into a variable-aspect container — it measures its box (seeding from
  `clientWidth` on mount, then a `ResizeObserver`), **clamps the computed image box to the available width/height**, and clips (`overflow-hidden`) so a
  wide (16:9) sidebar preview can never overflow its panel. The sidebar preview uses the live draft
  orientation for instant feedback on rotate clicks. It also takes an optional **`blurhash`** placeholder
  rendered behind the image (faded out on load) and shown on its own while `src` is still absent — the
  **Lightbox** passes the picture's blurhash so it shows immediately, before the `large` URL and image
  arrive, instead of a blank/spinner.
- **Infinite-scroll dedup:** `PhotoGrid` dedups the flattened pages by `id` before rendering — as new pictures shift pagination, consecutive pages can
  re-emit an already-seen item, which would otherwise render a duplicate card (and look doubly-selected if it was selected).
- **Single mapping per share:** `useShareMappings.addMapping` deletes any existing mapping first; the tagging `MappingEditor` hides already-mapped
  shares.
- **Cross-links:** right-panel tag → sets the `tag` filter; a provenance source badge → `/tagging/:source_id` (or `panel=incoming` + `share` highlight
  for an `incoming_share` source); a "Shared with you" tag → `tag` filter + `panel=incoming` + highlights the matching card in `IncomingSharesList`.
- **Search:** there is **no free-text/filename search** (a client-side filename filter was removed as misleading — it only matched already-loaded
  items). The **capture-date range** filter lives in the Filters menu (shared `DateRangePicker`, `date` mode); the picker works in `YYYY-MM-DD` and
  `FilterControls` maps the bounds to RFC3339 UTC (`T00:00:00Z` / `T23:59:59Z`) for the `captured_after`/`captured_before` wire params.
- **Gallery selection (touch vs desktop):** desktop uses click (single) / ⌘-click (toggle) / shift-click (range). Touch has no modifier keys, so
  `PhotoCard` detects a **long-press** (`450 ms`, cancelled if the finger moves > `10 px` so scrolling still works) that enters
  `selection.multiSelect` mode via `enterMultiSelect`; while it is on, a plain tap **toggles** a photo (and the selection circle shows on every card)
  rather than replacing the selection. Deselecting the last photo (or `clear()`) exits the mode. The long-press swallows the synthetic click that
  follows pointer release so the photo isn't immediately toggled back off. A normal single tap (not in multi-select mode) selects the one photo and,
  on
  mobile (`useIsMobile`), opens the right selection drawer (`openMobileDrawer('right')`). **Double-tap-to-open-lightbox is disabled on mobile**
  (it used to fire select + open at once); full-screen is reached from the sidebar preview instead.
- **Selection descriptor & action bar (feature 14):** the `selection` store is the `PictureSelection` descriptor (query + include/exclude deltas), not
  a flat id list. A grid card's checkmark is `isMemberSelected(...)` (in select-all mode a visible card is a query member, so selected unless
  `excludeIds`-listed). `PhotoGrid` renders `SelectionActionBar` (fixed, bottom-centre) on **desktop and mobile** whenever more than one picture is
  selected (hidden under the mobile drawer): resolved count, **Select-all** (adopts the view `PictureFilter`; `⌘/Ctrl+A` does the same), **Invert**
  (query mode), **Batch actions** (surfaces the right panel), **Clear**. **Any gallery view change** (tag / scope / sort / hierarchy dir) clears the
  selection — `PhotoGrid` clears on the `selectionFilter` signature — so a select-all's membership can't go stale and an explicit selection never
  lingers on an unrelated view. Batch writes resolve **at apply time** server-side; every one opens a `BatchConfirmDialog` that runs the endpoint's
  `dry_run` to preview the exact effect, so the previewed count can't diverge from the apply. (The grid `ul` is `select-none` so a shift-click range
  doesn't highlight the cards as text.)
- **Transparency backdrop:** images render over a **`.bg-checkerboard`** utility (`index.css`, two-tone diagonal-gradient grid using `--checker-1/2`,
  themed for dark/light) so transparent PNG regions read as transparent. In the grid the blurhash is a load-time placeholder that **fades
  to `opacity-0`
  once the thumbnail loads** (`PhotoCard` `loaded` state) — otherwise transparent areas would show the (opaque) blurhash instead of the checkerboard.
  `OrientedContainImage` paints the checkerboard on the exact image box (so the lightbox keeps a black letterbox around it).
- **Sensitive actions** (revoke / reject / delete) are gated by `ConfirmDialog`, which also accepts **Enter to confirm**
  regardless of which footer button has focus (Radix defaults focus to Cancel). The single exception is the keyboard
  trash shortcut: **Delete/⌘+Backspace** on a single selected picture (`SelectionPanel`) or the picture open in the
  `Lightbox` trashes it directly, skipping the dialog — a deliberate keystroke doesn't need the mouse-click confirm
  gate. Both shortcuts ignore the keypress while focus is in a text input and no-op on an already-trashed picture.
- **Upload flow:** `UploadDialog` (rendered once in `AppShell`) is triggered via `useUploadStore`. The `TopBar` Upload button and the `GalleryPage`
  full-page drag zone both call `openDialog(files?)`. **Files and directories** can be dropped or picked — dropped dirs are walked recursively via the
  `webkitGetAsEntry` filesystem-entries API and the folder picker uses a `webkitdirectory` input; in both cases **hidden files/dirs (dotfiles) are
  excluded** (`lib/uploadFiles.ts` — `filesFromDataTransfer`/`isHiddenFile`, shared with the gallery drop zone). The row previews are **lazy** (an
  `IntersectionObserver` only mints each `object URL` when its row nears the viewport — creating 1k object URLs up front froze phones). On submit each
  slot runs an **end-to-end per-file pipeline** with **bounded concurrency** (`UPLOAD_CONCURRENCY = 4`): it hashes the file's SHA-256
  (`crypto.subtle.digest`, lowercase hex — the same digest the worker produces; buffered in memory, hence the small bound), then **re-presigns
  just-in-time** (`POST /uploads/batch`, one file per call, with `initial_tags` + the import label) **right before** uploading — so a presign can't
  expire while earlier files in the batch are still transferring, and a **retry mints a brand-new URL** (the hash is reused, only the presign is
  fresh).
  A slot that comes back `duplicate: true` (the hash already matched an existing owned picture) is **not** uploaded — it shows an **amber check**
  ("Already in your library") and the backend has already assigned the initial tags to the existing picture. The per-call import label
  (`makeUploadLabel()` → `Uploaded.YYYY_MM_DD_HH_MM`, fixed once per batch so retries reuse it): the backend tags new uploads `Uploaded.…` and
  duplicates
  `Uploaded_….AlreadyExisting[.Deleted]` (feature 15). Trashed duplicates are **no longer auto-restored** — they come back `was_deleted: true`, and the
  completion screen shows an **import summary** (how many uploaded / already-existed / were-in-trash, each with its tag, plus a total) and a **Restore N
  deleted from trash** button (`restorePicture` per id). Non-duplicate files upload to S3 (via `XMLHttpRequest` for per-file progress) and call
  `POST /uploads/{id}/complete` immediately per file as its S3 PUT finishes — not after all files; `initial_tags` + `upload_label` are passed on the
  complete body and assigned atomically server-side. The backend wakes the pipeline through its debounced window, so per-file completions coalesce
  into a
  single run with no client-side defer/wake bookkeeping. An **overall progress bar + status line sit above the scrollable file list** (always
  visible);
  the percentage averages every slot's progress (settled = 100, in-flight = its S3 %) and is **floored to one decimal** (e.g. `99.5%`). Failed
  (network-errored) items carry a per-row **Retry** and the completion summary offers **Retry N failed**; a retry resets those slots to `pending` (the
  error count drops to 0 and the retry buttons hide while it reruns) so it reads like a fresh restart of only the failed files. Pictures **and tags**
  queries are invalidated when uploads settle, then **again ~1.5 s later** (uploaded pictures may pick up pipeline tags that land asynchronously).

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
