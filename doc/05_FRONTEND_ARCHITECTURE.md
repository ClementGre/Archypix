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
`.light` class on `<html>` (toggled by `stores/theme.ts`). The `dark:` utility variant is redefined in `index.css` (`@custom-variant dark`) to follow
that `.light` class rather than the browser's `prefers-color-scheme`, so `dark:` means "in-app dark mode" (applies when `<html>` is not `.light`). The
repo runs a code formatter — match surrounding style; don't fight it.

---

## 3. Connection model (federated)

There is **no `VITE_API_BASE_URL`**. Flow:

1. **Bootstrap + resolution** (`api/resolve.ts`, feature 25): `getResolverInfo(domain)` calls
   `{domain}/archypix-resolver/info` → `{ is_resolver, api_url }`. `resolveConnection(username, instance)`
   returns `{ backendUrl, isResolver, resolverUrl }`: on a standalone instance `api_url` **is** the
   backend (no user resolution); on a resolver-fronted one it additionally hits
   `{instance}/archypix-resolver/resolve?user=&domain=` (one call, replaces the old WebFinger query) for
   the owning `backend_url`. The `.well-known/webfinger` path is gone.
2. **Login** (`api/auth.ts` → `login`) resolves the connection, POSTs `/api/auth/login` at `backendUrl`,
   stores `{ accessToken, refreshToken, backendUrl, instance, isResolver, resolverUrl }` in the auth
   store, then loads `/api/auth/me`. `isResolver` gates the **Fleet dashboard** entry in the `TopBar`.
   Both **login** (`LoginPage`) and **logout** (`TopBar`) call `queryClient.clear()` so no cached query
   from a previous session ever bleeds into the next user's view.
3. **Authenticated calls** go through `api/client.ts` → `apiClient` (a single axios instance). Its request interceptor reads `backendUrl` +
   `accessToken`
   from the auth store **at request time**, so every call targets the right instance. The response interceptor does a **single-flight refresh** on 401
   (dedup'd across concurrent 401s); if refresh fails it clears the session and `ProtectedRoute` redirects to `/login`.
4. **Registration** (`api/auth.ts` → `register(payload, domain)`) bootstraps the domain, then `POST`s to
   `{api_url}/api/public/register` — served by a standalone backend directly and by a resolver under its
   `/archypix-resolver` prefix, so the frontend never has to know the topology. `domain` defaults to the
   global domain but the register page lets the user target a custom instance (same handle control as login).

**Env** (`.env`, all `VITE_`-prefixed, documented in `.env.example`): `VITE_GLOBAL_DOMAIN`, `VITE_USE_HTTPS`. Resolved in
`lib/constants.ts` (`GLOBAL_DOMAIN`, `USE_HTTPS`, `SCHEME`, `originFor(domain)`). Cross-instance picture fetching relies on dev `CORS_ORIGINS=*`.

The **login and register pages** share the handle as an editable control: `@<username>` + a click-to-edit instance field defaulting to
`GLOBAL_DOMAIN`,
so a user can authenticate or register against any instance. The chosen instance is persisted (`getPreferredInstance`/`setPreferredInstance` in
`lib/constants.ts`, `localStorage` key `archypix_instance`) so the two pages stay in sync. Both render `InstanceHealthWarning`
(`components/common/`, feature 25), which actively **pings** `{instance}/archypix-resolver/info` and shows a message only on a *real* problem —
unreachable, or reachable-but-CORS-blocked (a normal fetch that resolves ⇒ CORS ok; a throw + a `no-cors` probe that succeeds ⇒ CORS blocked;
both throw ⇒ down) — and nothing when the instance is healthy.

The **register page** keeps the account/instance editor (and the health warning) visible even when the chosen instance's registration is closed
(invite-only, no valid invite), so the user can switch to another instance without going back to login (feature 25).

---

## 4. Routes (`src/App.tsx`)

| Path                                 | Page                | Auth                 | Notes                                                                                                                                                     |
|--------------------------------------|---------------------|----------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|
| `/login`                             | `LoginPage`         | public               | WebFinger login + instance switcher                                                                                                                       |
| `/register`                          | `RegisterPage`      | public               | instance switcher (stays editable when registration is closed) + live health/CORS ping, then auto-logs in                                                 |
| `/`                                  | `GalleryPage`       | required             | the main three-pane workspace                                                                                                                             |
| `/tags`                              | `TagsPage`          | required             | placeholder (tag tree lives in the gallery panel)                                                                                                         |
| `/tagging`                           | `TaggingPage`       | required             | tagging-services editor                                                                                                                                   |
| `/tagging/:id`                       | `ServiceEditorPage` | required             | single tagging-service editor                                                                                                                             |
| `/shares`                            | `SharesPage`        | required             | placeholder (share UI lives in the gallery panel)                                                                                                         |
| `/settings`                          | `SettingsPage`      | required             | **Profile** page — account (profile + versioning + retention, one explicit Save), storage, invites + invitation graph (via user menu, labelled "Profile") |
| `/admin`                             | `AdminPage`         | admin only           | tabs: Overview / Users / Jobs / Shares / **Settings** / **Routines** / **Invites** (+ Fleet link, cache-clear refresh)                                    |
| `/admin/resolver`                    | `ResolverAdminPage` | **resolver session** | fleet dashboard — operator-token login, not `ProtectedRoute` (feature 24)                                                                                 |
| `/s/:global_domain/:username/:token` | `PublicSharePage`   | **public**           | link-gated public share (feature 27): resolves the owner backend from the URL, renders the gallery/lightbox/detail; no login required                     |
| `*`                                  | → `/`               | —                    |                                                                                                                                                           |

`ProtectedRoute` (`components/layout/ProtectedRoute.tsx`) gates auth and (with `adminOnly`) the admin role. Authenticated routes render inside
`AppShell` (unified `TopBar` + routed `<Outlet/>`). **There is no `/photos/:id`** — full-size viewing is the `Lightbox` carousel and details live in
the
right panel. **`/admin/resolver`** is deliberately **outside** `ProtectedRoute`: the resolver operator token is a separate credential from user auth
(feature 24), so `ResolverAdminPage` renders its own operator-token login when the `resolverAuth` store is empty and the tabbed fleet dashboard once a
session exists.

---

## 5. State management

### Zustand stores (`src/stores/`)

| Store             | Shape                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Persistence (`localStorage`)                                                   |
|-------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------|
| `auth.ts`         | `user, accessToken, refreshToken, backendUrl, instance` + setters/`clear`                                                                                                                                                                                                                                                                                                                                                                                                                                     | `archypix_auth`                                                                |
| `resolverAuth.ts` | resolver-operator session (feature 24) `sessionToken, refreshToken, expiresAt` + `resolverUrl` (the connected resolver's `api_url`, feature 25) + `setResolverUrl`/`setSession`/`clear` — **separate** from user auth; the `resolverClient` axios instance bears it, sets its `baseURL` to `resolverUrl` **at request time** (so an operator can target any resolver, not just the global domain), and single-flight-refreshes on 401; `ResolverAdminPage` schedules a background refresh before `expiresAt`. | `archypix_resolver_admin`                                                      |
| `ui.ts`           | `leftSidebarOpen, rightSidebarOpen, leftSidebarWidth, rightSidebarWidth, rowHeight, tagProvenance` + actions (`setLeftOpen/setRightOpen/setLeftWidth/setRightWidth`, clamped to `[SIDEBAR_MIN, SIDEBAR_MAX]`)                                                                                                                                                                                                                                                                                                 | `archypix_ui`                                                                  |
| `theme.ts`        | `theme: 'dark' \| 'light'` (applies/removes `.light`); `initTheme()` at boot                                                                                                                                                                                                                                                                                                                                                                                                                                  | `archypix_theme`                                                               |
| `selection.ts`    | the feature-14 **selection descriptor** `query: PictureFilter \| null, includeIds, excludeIds, anchor, multiSelect` (explicit mode = `query null`; select-all = an adopted view `query` + `excludeIds`; helpers `isMemberSelected`/`toApiSelection`/`hasSelection`/`isSingleSelection`; click / ⌘-toggle / shift-range / ⌘A; `multiSelect` = touch long-press mode, see §9)                                                                                                                                   | none (session only)                                                            |
| `upload.ts`       | `open, initialFiles, openDialog(files?), closeDialog` — upload dialog trigger shared by `TopBar` and `GalleryPage`                                                                                                                                                                                                                                                                                                                                                                                            | none (session only)                                                            |
| `lightbox.ts`     | Lightbox chrome: top-bar / carousel visibility kept **separately per fullscreen vs non-fullscreen** (`toggleTopBar`/`toggleCarousel` flip the current mode's flag), `fullscreen` (mirrors `document.fullscreenElement`), `originalQuality` (session-only, defaults off — presign the `original` instead of `large`); `topBarVisible`/`carouselVisible` selectors resolve the mode.                                                                                                                            | `archypix_lightbox` (visibility flags only; quality + fullscreen session-only) |
| `imageCache.ts`   | Per-picture registry of image URLs + which variants the browser has actually **loaded** (`record`/`recordImage`, `bestLoaded(entry, cap?)`). Lets the carousel/lightbox/sidebar reuse an already-loaded higher-or-equal variant with no new presign, and paint a lower-res one as a progressive placeholder.                                                                                                                                                                                                  | none (session only)                                                            |

`hooks/usePersistentBool.ts` persists individual booleans (used for foldable detail-section collapse under `archypix_ui_section_<id>`).

### URL as view state (`hooks/useGalleryParams.ts`)

The gallery view lives entirely in the URL so it is shareable and back/forward-friendly. `useGalleryParams()` returns typed `params`, derived
`filters` (for `usePictures`), and `update(patch, { replace })`. Params (defaults are **omitted** from the URL):

| Param                 | Meaning                                                                                                                                                |
|-----------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------|
| `tag`                 | active (primary) tag filter (wire form) — set by a plain tag click                                                                                     |
| `inc` / `exc` / `exa` | extra compound-filter tag sets (comma wire paths): include / exclude / exact (strict) — built from the tag sidebar `…` menu                            |
| `scope`               | `all` \| `owned` \| `shared`                                                                                                                           |
| `trash`               | trash membership: `exclude` (default, omitted) \| `include` \| `only` (trash view) — a filter over the main view, set by the grid-header `TrashToggle` |
| `sort`                | `captured_at` (default) \| `ingested_at` \| `updated_at` \| `file_size` \| `filename`                                                                  |
| `order`               | `asc` \| `desc`                                                                                                                                        |
| `after` / `before`    | capture-date bounds (ISO)                                                                                                                              |
| `panel`               | active left tab: `tags` \| `incoming` \| `outgoing` \| `hierarchies`                                                                                   |
| `share`               | incoming share id to highlight (cross-link target)                                                                                                     |
| `hierarchy`           | active hierarchy id — center grid browses it (via `browse`) instead of the flat list                                                                   |
| `hpath`               | directory path within the active hierarchy (slash-separated names, `''` = root)                                                                        |
| `hedit`               | hierarchy id whose config editor occupies the center view (overrides the grid)                                                                         |
| `view`                | open the Lightbox on this picture id (set by `PhotoGrid`)                                                                                              |

---

## 6. Data layer

One file per domain under `src/api/` (typed axios wrappers using `apiClient`), with matching hooks under `src/hooks/` (TanStack Query). Types live in
`src/lib/types.ts`; query keys are centralized in `src/lib/constants.ts` (`queryKeys`).

| Domain        | `api/*`                                                                                                                                                                                                                                                                                                                                                                                  | `hooks/*`                                                                                                                                                                                                                                                                                |
|---------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| auth          | `auth.ts` — `login, logout, register, fetchMe`                                                                                                                                                                                                                                                                                                                                           | (imperative; not query-backed)                                                                                                                                                                                                                                                           |
| pictures      | `pictures.ts` — `listPictures, getPicture, getPictureUrl, editPicture, overrideExif, setCreator, trashPicture, restorePicture, getJob, beginUploadBatch, completeUpload`; **batch (feature 14)** `aggregatePictures, batchEditExif, batchTrash, batchRestore` (all over a `PictureSelection`, `dry_run`-aware)                                                                           | `usePictures` (infinite, `thumbnail:'medium'`, page 50), `usePictureEdit.{useEditExif, useOverrideExif, useSetCreator, useTrashMutations}`; `useAggregate(selection, sections)` (debounced, lazy sections) + `useSelectionCount`; `useBatch.useBatchMutations` (trash/restore/tags/exif) |
| tags          | `tags.ts` — `listAllTags, listPictureTags, listPictureTagsWithSources, batchEditTags` (selection + `dry_run`-aware)                                                                                                                                                                                                                                                                      | `useTags` — `useAllTags, usePictureTags, useBatchEditTags`                                                                                                                                                                                                                               |
| shares        | `shares.ts` — `list/accept/reject/revoke/createOutgoing`                                                                                                                                                                                                                                                                                                                                 | `useShares` — `useIncomingShares, useOutgoingShares, useShareMutations`; `useShareMappings`                                                                                                                                                                                              |
| public shares | `publicShares.ts` (feature 27) — public (no-auth) `resolvePublicBackend/getPublicMeta/unlockPublicShare/listPublicPictures/getPublicPictureUrl/getPublicPictureDetail/publicAggregate/publicUploadBatch/publicCompleteUpload` (own axios instance, no interceptors) + authed `saveCopyFromPublic/subscribeToPublic` + owner `list/create/update/revoke/trashContributions` (`apiClient`) | `usePublicShares` (owner: `usePublicShares/usePublicShareMutations`); the public page uses inline `useInfiniteQuery`/`useQuery`                                                                                                                                                          |
| tagging       | `tagging.ts` — service + rule/segment/mapping CRUD, `reorderServices`                                                                                                                                                                                                                                                                                                                    | `useTaggingServices` — `useTaggingServices, useTaggingService, useTaggingMutations`                                                                                                                                                                                                      |
| hierarchies   | `hierarchies.ts` — CRUD + `getHierarchyTree`, `browseHierarchy`, `getWebdav`/`regenerateWebdavToken`/`setWebdavUseRedirect`                                                                                                                                                                                                                                                              | `useHierarchies` — `useHierarchies, useHierarchy, useHierarchyTree, useHierarchyBrowse, useHierarchyMutations, useWebdav, useWebdavMutations`                                                                                                                                            |
| settings      | `settings.ts` — `getSettings, updateSettings` (versioning + `trash_retention_days`)`, updateProfile`                                                                                                                                                                                                                                                                                     | `useSettings` — `useSettings, useUpdateSettings, useUpdateProfile`                                                                                                                                                                                                                       |
| admin         | `admin.ts` — instance/stats/users/jobs/shares + **runtime config** `getAdminSettings/patchAdminSetting/resetAdminSetting`, `getAdminRoutines/triggerAdminRoutine`                                                                                                                                                                                                                        | `useAdmin` — `…, useAdminSettings/useAdminSettingMutations, useAdminRoutines/useTriggerRoutine`                                                                                                                                                                                          |
| invites       | `invites.ts` — `listInvites/mintInvite/revokeInvite, getInvitations` + public `previewInvite/getRegistrationInfo` (feature 23 §6; backend forwards to the resolver in resolver mode)                                                                                                                                                                                                     | `useInvites` — `useInvites, useInvitations, useInviteMutations, useRegistrationInfo`                                                                                                                                                                                                     |
| resolver      | `resolverAdmin.ts` (feature 24; `resolverClient` base is the chosen resolver's `api_url`, feature 25) — `login/refresh, getOverview/getBackends/setCapacity, getSettings/patchSetting/resetSetting, listInvites/mintInvite/revokeInvite, getConfigMatrix/patchConfigMatrix, proxy(backDomain,…)`                                                                                         | `useResolverAdmin` — `useResolverSession, useResolverOverview, useResolverBackends, useResolver{Settings,Invite,Capacity}Mutations, useConfigMatrix/useConfigMatrixPatch`                                                                                                                |

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
Sort offers Date taken / added / modified, File size, Name — default **Date taken** (`captured_at`); Filters folds in scope
and a **capture-date range** using the shared `DateRangePicker` calendar — no tag chips here, those live in the centre `TagFilterBar`),
`TrashToggle` (`components/photos/`, a three-state segmented control — **Photos** (hide trashed, default) / **All** (include trashed) / **Trash**
(trashed only) — pinned to the right of the grid header, writing the `trash` URL param; the trash is a **filter over the main view**, not a
separate page),
`TagFilterBar` (`components/tags/`, a breadcrumb-style bar atop the flat grid showing the active include / `=`-exact / `⦸`-exclude tags as chips,
each with a switch-include↔exact control + remove, plus Clear; shares the grid-header row with `TrashToggle` — the flat tag chips / the hierarchy
breadcrumb on the left, the trash toggle on the right), `Lightbox` (full-screen viewer driven by the `view` param;
←/→/Esc, plus **Delete/⌘+Backspace trashes the picture in view immediately, no confirm dialog** — both that shortcut
and the header's trash button (via `ConfirmDialog`) then **advance to the next picture (or previous if it was last)
instead of closing**, only closing when no picture remains;
**portaled to `document.body`** so it paints above the mobile sidebar drawer while the trash confirm dialog still stacks
on
top; **click the backdrop outside the image to close** — and closing **selects the viewed picture** (opening the right drawer on mobile) so the user
lands on its specs. The top bar sits **in flow when pinned** (the image never goes under it) and becomes a slide-in **overlay** only when hidden (
revealed on a
top-edge hover); it carries the filename, `index/total`, **file size + mime + owner handle + trash / owner-deleting badges**,
**download-original**, copy-to-library (received), a trash (`ConfirmDialog`)/restore action, and rotate-left/right (over the image, auto-committing
orientation via `useExifDraft`). Three chrome toggles (state in `stores/lightbox.ts`): **top-bar visibility**, **carousel visibility** (both persisted
per fullscreen-vs-normal mode), and **original-quality** (session-only — presigns the `original` instead of the default `large`). A **fullscreen**
button
enters the browser fullscreen API; while the top bar is hidden the whole chrome hides, and each control re-appears while the mouse is near its edge
(top → bar, left/right → nav arrows, bottom → rotate). Still images support **ctrl/⌘ + wheel zoom-to-cursor and drag-to-pan** (`ZoomableArea`,
double-click toggles), and reuse an already-loaded lower-res variant (`stores/imageCache.ts`) as a progressive placeholder (`OrientedContainImage`'s
`placeholderSrc`, painted eager so a browser-cached thumbnail shows instantly with no blurhash flash). `LightboxCarousel` is the optional bottom
filmstrip: the current picture centres (half-width end spacers let the first/last thumb reach the centre), clicking or sliding/scrolling a thumbnail
into the centre changes it, thumbnails reuse already-loaded / list `thumbnail_url` images before requesting a `small` presign, and image work is
**lazy** (per-thumb `IntersectionObserver`) so a large library doesn't presign every thumbnail; the viewer also pages in more grid items (`loadMore`)
as it nears the end of what's loaded), `SelectionPanel`
(right panel; see §8), `PhotoCard` (also surfaces trash state — dimmed + a corner trash chip when `deleted_at` is set — a **red** owner chip with
an alert icon when a received picture's `owner_deleted_at` is set, and — in the **trash-only** view (`showPurgeCountdown`) — a red purge-countdown
overlay ("Deletes in 12 days") on owned trashed pictures, the deadline derived as `deleted_at + trash_retention_days`), `UploadDialog` (batch upload
with drag-and-drop, per-file progress, and initial
tag assignment — see §9; its upload backend + which sections show (tags / storage / import summary /
contributor name / dedup message) come from an `UploadSource` prop, default = the authenticated library
upload, so the public share page reuses the same dialog for anonymous contributions), `MediaPlayer` (Vidstack wrapper — picks the default video/audio
layout from the picture's mime; used by the
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
fetching), `CreatorField` (feature 26 "Created by" line: parses the resolved `creator` by leading sigil — `@user:domain` identity handle / `#name`
"Created by {name}" + a **"public share"** chip / plain — click-to-edit through `useSetCreator` via the shared `ContactInput` (`common/`, plain text
or
an autocompleted+resolver-verified `@user:domain`; `#` blocked); owned pictures edit the authoritative `creator_value` with "reset to owner default",
received pictures the `creator_override` with "reset to original" + an "overridden" badge),
`ExifInlineEditor` (presentational inline per-field EXIF
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
**`…` menu** with toggle actions **Include / Include exactly / Exclude** (writing the `inc`/`exa`/`exc` params), plus **Share this tag…** (opens a
pre-filled `CreateShareDialog`) and **Rename tag…**, and **⌘/Ctrl-click** quick-toggles a
tag in the include set to build "X and Y" fast; "Include exactly" is the strict/no-descendant mode (backend `exact`). The tree only **highlights**
rows by state — emerald (included/exact, `=` icon) or struck-through red (excluded, `⦸` icon); the active filter itself is surfaced in the centre
`TagFilterBar` breadcrumb (not in the tree)), `TagPicker` (autocomplete over existing tags + create-new; `allowProtected` prop — see §9; optional `trigger` prop
to
render a custom trigger, e.g. the small **+** button in the details-panel Tags section header. Each list row carries a **›** button (and **Tab**
autocompletes the highlighted tag) that fills the field with `<tag>/` so the user can append a child without retyping — e.g. autocomplete `/Event`
then type `Birthday`. Input is **sanitized live**: accents stripped, spaces/`-` → `_`, `.`/`\` → `/` with an **amber** note of what was replaced, and a
**red** warning for a reserved `SharedToMe` prefix or any still-invalid character).

**`tagging/`** — `TaggingPage` (titled **"Tagging services"**; header has a **Force run** button — `POST /pictures/pipeline/wake` — for debugging)
composes `PipelineList` (rule + segmentation services — the "queries" — shown **first**) then `SharedMappingSection`
(shared-tag-mapping services, which still **run first** in the pipeline, in a **collapsed-by-default `Section`** below the queries so the user reads
the rules/segments before expanding the mappings).
`PipelineList` (**@dnd-kit reorder that never includes shared_tag_mapping ids**) of `ServiceCard`s (a compact row —
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
("No message" when null), plus an optional `footer` slot. `IncomingSharesList` keeps flat rows (accept / reject(confirm) / view-photos; a share maps
to one or more
local tags via `useShareMappings` — each shown as a removable chip plus an add control) and, when the sender allows it, a **Share back** button in the
popover footer that opens a controlled
`CreateShareDialog` pre-targeted at the sender and pre-filled with that share's first mapped local tag (if a `SharedTagMappingService` mapping exists;
still
editable). `OutgoingSharesList` groups by tag via a factorized
`GroupedShareRow` reused across all three sections — the group header shows the most common `name` with a "(and N others)" suffix when names differ
(`summarizeNames`), the per-recipient details living in the popover, plus confirm-revoke. Both lists cross-reference the other direction (incoming ↔
outgoing) to resolve the ShareBack provenance label. `CreateShareDialog` creates **one share per recipient**: an optional **Share back of** combobox
(lists the user's live incoming shares; selecting one marks the share a ShareBack — sends `shareback_of` = that incoming share's `outgoing_share_id`
and
locks the recipient to its sender), common `name` (≤ 64) / `message` (≤ 1000) / tag / ShareBack / future inputs and a recipient list of
**`ContactInput`** rows (`common/`, identity-only — `@`-prefixed autocomplete over the user's share partners, an instance-less `@alice` defaulting to
the global domain; a button adds rows); on submit it fires one request per recipient sequentially with per-recipient progress icons (mirrors
`UploadDialog`). **A failed recipient stays editable and gets a per-row Retry** (and the footer button re-runs all not-yet-done rows), so a single bad
handle doesn't force re-entering the others. It supports a controlled `open`/`onOpenChange` (with `showTrigger=false`) and
an
`initialTag` prop so the ShareBack button **and the tag tree's `…` → "Share this tag…"** can drive it pre-filled. `ShareStatusBadge` maps status →
coloured pill (used inside the popover). **Public share links (feature 27):** the Outgoing panel header is a
single row of two equal-width compact buttons — **Share tag** (`CreateShareDialog`) and **Public share**
(`PublicShareDialog`), both driven controlled from the header (no "Outgoing" label). `PublicShareDialog` is **create-or-edit** (`share` prop
prefills + switches to
`update`; the tag is immutable on edit; a "Change/Keep current" password control), `initialTag`-driven
(also from the tag tree `…` menu). `PublicLinksManager` is a collapsed-by-default `Section` listing the
active links, each row with copy-link / **edit** / revoke and a `PublicShareInfoPopover` (reuses
`ShareInfoPopover`'s `FlagChip`/`DetailRow` — permissions, tag, password, counts, timestamps, message).

**`public/`** (feature 27, `/s/:global_domain/:username/:token`) — the unauthenticated public share page,
**factorized to reuse the authenticated gallery** rather than rebuild it. `PublicSharePage` resolves the
owner backend from the URL (`resolvePublicBackend`), fetches the meta, renders the password gate when
required, then the layout via a `PublicShareProvider` (`components/public/context.ts`) **and** a
`PictureSourceProvider` (see `components/photos/pictureSource.ts`) carrying a read-only
`usePublicPictureSource` (token-gated presign/detail on the owner backend, `readOnly`, `canDownload =
allow_originals`, namespaced query keys). The page reuses the shared **`PhotoCard`**, **`Lightbox`** (+
`LightboxCarousel`), **`SidePanel`** (resizable/mobile right panel), **`OrientedContainImage`**, and the
factored **`ThumbnailSizeSlider`**. `PublicTopBar` (mirrors the app `TopBar` chrome — brand, theme toggle,
details-panel toggle, user menu / Sign-in + Create-account — with the share info standing in for the
breadcrumb; **Upload** + **Add to my account** menu (*Save selected copies* / *Convert to a share on your
account*), all **hidden from the album's own owner**), `PublicGallery` (justified `PhotoCard` grid driving
the shared `Lightbox` via the `view` param, and using the **global feature-14 `selection` store** + the
shared **`SelectionActionBar`** — the same click / ⌘-toggle / shift-range / long-press semantics, the
floating bottom bar, and ⌘/Ctrl+A; select-all/invert run in **explicit** mode over the loaded ids since a
token-gated album has no server `PictureFilter`, and the store is cleared on enter/leave so it never bleeds
between the app and a public album), `PublicDetailPanel` (preview + sigil-parsed creator + read-only EXIF +
download/save-a-copy, or a batch EXIF aggregate), `PublicStatusBar` (share spec + the shared zoom slider), and the shared **`UploadDialog`** driven by
a
token-gated `usePublicUploadSource` (the `UploadSource` abstraction — contributor name, forced album tag,
no storage/import/restore surface, dedup message "The owner already has this picture"). A `noindex` meta tag
is injected. **`Lightbox`/`LightboxCarousel` read a `PictureSource` from context** (default
`AUTH_PICTURE_SOURCE` = the existing keys + full read/write; the public source is read-only), so the one
viewer serves both surfaces with no duplicate code.

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
per-node card; add/remove/reorder of `mirror`/`query`/`static`/`drop` nodes with their kind-specific fields and an Advanced disclosure for per-node
naming/safeDeleteMode and the **write-back tri-state** `inherit|on|off` — feature 18; the master switch + parent chain are threaded down as a `wb`
context so it labels "Inherit (on/off)", disables under a master-off ceiling, and gates the safe-delete control on the node's effective write-back;
`mirror` fields include `maxDepth`/`deeperMode` and foreign-tag excludes; `drop` is a name + assign-tags inbox editor), `WriteBackEditor` (query-node
write-back op-lists with a "suggest from predicate" helper — enabled on `matchUntagged` nodes too with a free-form op-list + the "can't guarantee
untagged" warning, and an inactive-when-write-back-off note), `TagListField` (chips + `TagPicker`, reused for include/exclude/collapsed/drop-assign),
`JsonConfigDialog` (raw `config` textarea; applies to the draft).

**`admin/`** (backend `/admin` + shared) — `OverviewTab`/`UsersTab`/`JobsTab`/`SharesTab`, plus the feature-23/24 additions: **`SettingsPanel`** (the
metadata-driven runtime-config editor — a filter box, then groups `FieldMeta[]` by `group`, renders a control per `kind` (bool→Switch, enum→Select,
numbers→**`NumberInput` with per-kind min/max/step** — u16 0–65535 etc., list→comma-string), a `FieldInfoPopover` (i) with the copyable env name /
type /
default / example / description, env-locked fields read-only with a badge, a provenance chip + reset-to-default on `db`-sourced fields, and a *
*collapsed
"Core (env-only)" section** for non-runtime fields tagged red **core** / **secret** (secret values arrive redacted); pointed at `apiClient`,
`resolverClient`, or the per-instance proxy), `RoutinesPanel` (live routine status + inline tuning via `SettingsPanel flat` + Trigger-now,
polls while open), `SettingsTab`, `InvitesTab`, and **`InvitesManager`** (shared by `/admin` + the Profile page + the resolver dashboard — mint form
adapts to the registration mode (open ⇒ a single **tracking** referral link `max_uses:null`, else allowed-uses/unlimited + expiry, resolver adds an
instance-pin picker), short grouped codes (`ABC-DEF-GHI`), copyable `/register?invite=` links, revoke; a `groupByCreator` mode groups the list under
per-user headers for the admin `/admin` Invites tab (`GET /api/admin/invites`, all local) and the fleet Invites tab). **Admin + Fleet dashboard live
in
the `TopBar` user dropdown** (not the icon nav) for admins.

**Admin transport abstraction** (`api/adminClient.ts`) — the admin surface is reached two ways: the user's `apiClient` (direct `/admin`) or a
per-backend **proxy client** that rewrites `/api/admin/*` → the resolver's `instances/{d}` delegation-replay path. Admin api fns take an
`AxiosInstance`;
`useAdmin` hooks read it (+ a cache `scope`) from `AdminClientProvider`/`useAdminClient`, so `AdminDashboard` (the whole Overview/Users/Jobs/Shares/
Settings/Routines tab set) renders identically for `/admin` and a fleet drill-down.

**`resolver/`** (feature 24, `/admin/resolver`) — `ResolverLogin` (resolver-domain switcher defaulting to the user's instance +
`InstanceHealthWarning`; bootstraps `/info` to find the resolver `api_url`, then operator-token → session, feature 25), `ResolverOverviewTab` (fleet
Σ + `BackendHealthList`
with
per-backend capacity %), `ResolverBackendsTab` (master list → a selected backend's **full `AdminDashboard`** proxied via delegation replay — the
second
tab bar; its capacity editor + fleet-side backend state — heartbeat / delegation expiry / reachability / version / last-selected — live in an injected
**Resolver** `extraTab` so they don't crowd the top), `ResolverUsersTab` (**reuses the backend `UsersTab` verbatim per reachable backend** inside a
proxy `AdminClientProvider`, so create/edit/quota/delete/audit route to the right instance), `ResolverConfigMatrixTab` (group-headed, diff-first
field×backend table with an info popover + per-field set-all fan-out), `ResolverSettingsTab`, `ResolverRoutinesTab` (reuses the shared
`RoutinesView`),
`ResolverInvitesTab` (grouped by creator + instance-pin). The header is compact with an auto-refresh toggle, a cache-clearing refresh, and a
back-to-{instance}-admin button when a user session exists. `AdminDashboard` takes an `extraTabs` callback slot so resolver-specific tabs inject
without
leaking into the shared dashboard.

**`common/`** — `ConfirmDialog` (AlertDialog wrapper gating sensitive actions), `ContactInput` (feature 26 — a `@user:domain`
contact autocomplete drawn from the user's incoming + outgoing share partners, with a best-effort existence check
(`checkIdentityExists`, `api/resolve.ts`) that hits `/archypix-resolver/resolve` **directly** — same path on resolver and standalone backend, no
`/info` bootstrap — distinguishing a resolver `404` (**"no account … on {domain}"**, the instance answered but the user is missing) from a connection
failure (**"{domain} is unreachable"**); it only checks/surfaces once a syntactically valid domain (`:` + a dot) has been typed (never for a
defaulted or half-typed instance; the check is advisory and never blocks submit); an `allowCustomValues` flag toggles between identity-only — a
hardcoded `@` prefix, used for share recipients — and plain-text-allowed with a leading `@` opening the autocomplete and `#` blocked, used for the
creator field; an `includeSelf` flag leads the suggestions with the logged-in user's own identity — on for the creator, off for shares),
`MapView` (shared imperative Leaflet map;
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
  a **"Created by" creator field** (`CreatorField`, feature 26 — sigil-parsed display, inline edit, owned "reset to owner default" / received
  local-override "reset to original"),
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
  width) — but reuses a higher variant already loaded (via `stores/imageCache.ts`) rather than a fresh `medium` presign; the **Lightbox** uses `large`
  (or `original` when the original-quality toggle is on). Presigned URLs are cached in Query (`['pictures','url',id,variant]`, ~10 min `staleTime`);
  additionally `stores/imageCache.ts` tracks which variant URLs the browser has **loaded** so the carousel/lightbox/sidebar can reuse them instantly.
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
  Trashing also **optimistically drops the id from every cached grid list** (`removePicturesFromLists` in `lib/invalidation.ts`,
  page/offset-agnostic —
  it filters the infinite-query pages directly rather than relying on a refetch, so a delete from the Lightbox or sidebar disappears instantly;
  skipped
  for views that show the trash (`trash: include | only`) where the item legitimately stays). The subsequent invalidation reconciles totals.
  Owned trash is purged after `trash_retention_days`; received trash is local-only. **The trash is a filter over the main gallery, not a separate
  page** (there is no `/trash` route): the grid-header `TrashToggle` selects **Photos** (`trash=exclude`, default) / **All** (`include`) / **Trash**
  (`only`), the backend does the filtering (`GET /pictures?trash=…` — no client-side `deleted_at` filtering), and trashed items render dimmed with a
  purge/owner-deletion badge inline in the same grid + `SelectionPanel` (single or batch **Restore** there). The Profile storage card's "Open trash"
  button deep-links to `/?trash=only`.
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
- **Multi-tag mapping per share:** one `SharedTagMappingService` per incoming share, but it may map to **several** local tags. `useShareMappings`
  exposes `addMapping` (appends a tag, creating the service on the first one), `removeTag` (drops a single tag — deleting the service once its last
  tag
  is removed), and `forShare` (one `ShareMapping` per assigned tag). The `MappingControl` in `IncomingSharesList` renders one removable chip per
  mapped
  tag plus an always-present add control; the tagging `MappingEditor` edits the full `assign_tags` list via `PUT …/config`.
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

## 11. Coding guidelines (agents)

- **Don't start or preview the frontend dev server yourself.** Only check that it builds (`npm run build`). The user can give feedback on frontend
  changes by running the app themselves.
- Keep code comments short and sparse — see the shared rule in doc/00_CODING_GUIDELINES.md.
- Keep documentation (this file included) up to date, matching the level of detail already present — don't add overly specific descriptions of a
  single change beyond what the rest of the doc covers.
- When editing an endpoint's request/response shape, check it against doc/06_API_REFERENCE.md.
- When completing a task, update doc/99_ROADMAP_MVP.md, and add things not yet implemented to it.
