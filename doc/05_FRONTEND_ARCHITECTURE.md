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
4. **Registration** (`api/auth.ts` → `register`) is auto-detecting: it tries the resolver `POST {global}/api/register`; on a 404 (a standalone backend
   has
   no such route) it falls back to `POST {global}/api/public/users`. `VITE_REGISTRATION_MODE` (`auto`|`resolver`|`standalone`) and
   `VITE_REGISTRATION_URL`
   override this.

**Env** (`.env`, all `VITE_`-prefixed, documented in `.env.example`): `VITE_GLOBAL_DOMAIN`, `VITE_USE_HTTPS`, `VITE_REGISTRATION_MODE`,
`VITE_REGISTRATION_URL`. Resolved in `lib/constants.ts` (`GLOBAL_DOMAIN`, `USE_HTTPS`, `SCHEME`, `originFor(domain)`). Cross-instance picture fetching
relies on dev `CORS_ORIGINS=*`.

The **login page** has the handle as an editable control: `@<username>` + a click-to-edit instance field defaulting to `GLOBAL_DOMAIN`, so a user can
authenticate against any instance.

---

## 4. Routes (`src/App.tsx`)

| Path           | Page                | Auth       | Notes                                             |
|----------------|---------------------|------------|---------------------------------------------------|
| `/login`       | `LoginPage`         | public     | WebFinger login + instance switcher               |
| `/register`    | `RegisterPage`      | public     | registers on the global domain, then auto-logs in |
| `/`            | `GalleryPage`       | required   | the main three-pane workspace                     |
| `/tags`        | `TagsPage`          | required   | placeholder (tag tree lives in the gallery panel) |
| `/tagging`     | `TaggingPage`       | required   | tagging-pipeline editor                           |
| `/tagging/:id` | `ServiceEditorPage` | required   | single tagging-service editor                     |
| `/shares`      | `SharesPage`        | required   | placeholder (share UI lives in the gallery panel) |
| `/settings`    | `SettingsPage`      | required   | profile + versioning mode (reached via user menu) |
| `/trash`       | `TrashPage`         | required   | placeholder                                       |
| `/admin`       | `AdminPage`         | admin only | placeholder                                       |
| `*`            | → `/`               | —          |                                                   |

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

| Param              | Meaning                                                              |
|--------------------|----------------------------------------------------------------------|
| `q`                | filename search (client-side filter — see §9)                        |
| `tag`              | active tag filter (wire form)                                        |
| `scope`            | `all` \| `owned` \| `shared`                                         |
| `deleted`          | include trashed (`1`)                                                |
| `sort`             | `ingested_at` \| `captured_at` \| `updated_at`                       |
| `order`            | `asc` \| `desc`                                                      |
| `after` / `before` | capture-date bounds (ISO)                                            |
| `panel`            | active left tab: `tags` \| `incoming` \| `outgoing` \| `hierarchies` |
| `share`            | incoming share id to highlight (cross-link target)                   |
| `view`             | open the Lightbox on this picture id (set by `PhotoGrid`)            |

---

## 6. Data layer

One file per domain under `src/api/` (typed axios wrappers using `apiClient`), with matching hooks under `src/hooks/` (TanStack Query). Types live in
`src/lib/types.ts`; query keys are centralized in `src/lib/constants.ts` (`queryKeys`).

| Domain   | `api/*`                                                                                                          | `hooks/*`                                                                                   |
|----------|------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| auth     | `auth.ts` — `login, logout, register, fetchMe`                                                                   | (imperative; not query-backed)                                                              |
| pictures | `pictures.ts` — `listPictures, getPicture, getPictureUrl, editPicture, getJob, beginUploadBatch, completeUpload` | `usePictures` (infinite, `thumbnail:'medium'`, page 50), `usePictureEdit.useEditExif`       |
| tags     | `tags.ts` — `listAllTags, listPictureTags, listPictureTagsWithSources, batchEditTags`                            | `useTags` — `useAllTags, usePictureTags, useBatchEditTags`                                  |
| shares   | `shares.ts` — `list/accept/reject/revoke/createOutgoing`                                                         | `useShares` — `useIncomingShares, useOutgoingShares, useShareMutations`; `useShareMappings` |
| tagging  | `tagging.ts` — service + rule/segment/mapping CRUD, `reorderServices`                                            | `useTaggingServices` — `useTaggingServices, useTaggingService, useTaggingMutations`         |
| settings | `settings.ts` — `getSettings, updateSettings, updateProfile`                                                     | `useSettings` — `useSettings, useUpdateSettings, useUpdateProfile`                          |

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
rendered inside `TopBar`), `Lightbox` (full-screen carousel driven by the `view` param; ←/→/Esc; `large` variant), `SelectionPanel`
(right panel; see §8), `UploadDialog` (batch upload with drag-and-drop, per-file progress, and initial tag assignment — see §9).
**`photos/detail/`** — `Section` (compact foldable section, collapse persisted per id), `ExifInlineEditor` (presentational inline per-field EXIF
editor:
blue-dot dirty indicator, per-field reset on hover, save button in section header, exif_sync_status badge, unit-prefixed/suffixed inputs; every
editable
row — including the exposure num/den pair on one line — is **click-to-edit** (shows formatted text, swaps to inputs on click); orientation is **not**
a row —
it is driven by the preview's rotate overlays), `DateTimePickerPopover` (shadcn Calendar + time input, auto-applies on change → `NaiveDateTime` string
`YYYY-MM-DDTHH:MM:SS`, **no timezone**; Clear resets and closes the popover), `GpsPickerPopover` (interactive map + manual lat/lng/alt inputs +
"current location" + Clear). The draft state is owned by `hooks/useExifDraft.ts` (shared between the editor and the preview's rotate buttons; re-seeds
from server
state on the picture's `id`+`updated_at` signature, exposes `set/setGps/reset/resetGps/rotate/save`). **Orientation is excluded from the manual
dirty/save flow** — `rotate` updates the draft for instant feedback and auto-commits the new orientation after a 700 ms debounce (
`set: { orientation }`),
so the rotate buttons never leave the EXIF section "dirty".

**Leaflet is loaded from CDN at runtime** (vanilla `leaflet` via a one-time injected `<script>`/`<link>`, no npm package) — the `react-leaflet`
wrapper
pulled a duplicate React copy under the project's mixed npm/pnpm `node_modules` and crashed with "Invalid hook call". Vanilla Leaflet has no React
dependency, so `GpsPickerPopover` drives the map imperatively in a `useEffect`.

**`tags/`** — `TagTree` (recursive hierarchy from `useAllTags`; click sets the `tag` filter; auto-expands ancestors of the active tag and scrolls it
into
view when it changes externally), `TagPicker` (autocomplete over existing tags + create-new; `allowProtected` prop — see §9; optional `trigger` prop
to
render a custom trigger, e.g. the small **+** button in the details-panel Tags section header).

**`tagging/`** — `TaggingPage` composes `SharedMappingSection` (shared-tag-mapping services in a **collapsed-by-default accordion, always first**)
then
`PipelineList` (rule + segmentation services, **@dnd-kit reorder that never includes shared_tag_mapping ids**) of `ServiceCard`s.
`RequiresExcludesEditor`
(gates as a **local draft committed on Save**), `RuleEditor`, `SegmentEditor`, `MappingEditor`, `DeleteServiceDialog` (promote-vs-remove),
`NewServiceMenu`.

**`shares/`** — `IncomingSharesList` (compact rows; **pending shares in a foldable `Pending` section at the top**; accept / reject(confirm) /
view-photos;
single local-tag mapping per share via `useShareMappings`), `OutgoingSharesList` (**pending shares in a foldable `Pending` section at the top**, the
rest
**grouped by tag**, per-recipient status + confirm-revoke), `CreateShareDialog`, `ShareStatusBadge`.

**`common/`** — `ConfirmDialog` (AlertDialog wrapper gating sensitive actions).

---

## 8. The gallery workspace

`GalleryPage` is a three-pane layout under the unified `TopBar`; each side panel is wrapped in `SidePanel` (resizable on desktop, overlay drawer on
mobile) and shown only when its `ui` store toggle is on:

- **Left** (`LeftPanel`): tabbed Tags tree / Incoming shares / Outgoing shares / Hierarchies (placeholder).
- **Center** (`PhotoGrid`): the justified grid; double-click opens the `Lightbox`; click an already-selected photo deselects it.
- **Right** (`SelectionPanel`): only mounted when a selection exists (returns `null` otherwise — its `SidePanel` wrapper additionally honours the
  `rightSidebarOpen` toggle). For a single selection: borderless thumbnail (click opens lightbox; received pictures get an `@owner:instance` label
  overlaid on the preview), filename + size/dimensions/mime inline, ingested/updated timestamps (formatted in the local timezone via
  `formatDateTime`),
  then foldable sections — **Tags** (chips; **+** add button and provenance toggle in the section header; provenance mode renders each path as a chip
  with colour-coded per-source mini-tags), **Shared with you** (sender handle + shared subpath, not the raw `SharedToMe.*` path), **Shared by you**,
  **EXIF** (inline-editable for owned pictures; the status badge flips to a green **modified** when there are unsaved changes), **Versions**. Clicking
  a
  tag filters by it and reveals it in the Tags tree (opens the left panel + Tags tab + expands/scrolls the tree). For multi-selection: batch tag-add.

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
- **EXIF editing:** owned pictures only (`picture.owner_username == null`). `useEditExif` POSTs the diff (`set`/`clear`), then polls `getJob`
  (1/2/4/8/15 s) while `exif_sync_status === 'pending'`.
- **Orientation rendering:** stored thumbnails/originals are raw pixels (orientation not baked in). `components/photos/OrientedImage.tsx` rotates them
  at display time from the picture's `orientation` (list item + detail). `orientedCoverStyle` produces the absolute positioning + transform that makes
  a
  90°/270° image fill a parent already laid out at the **display** aspect ratio (transposed via `displayDimensions`); it sets `max-w-none` to escape
  Tailwind preflight's `img { max-width: 100% }`, which would otherwise clamp the >100% width into a square. `OrientedImage` (used by `PhotoCard`,
  which
  also rotates the `Blurhash` placeholder with the same `orientedCoverStyle` so it lines up). `OrientedContainImage` measures its available box to
  *fit* a
  rotated image inside a variable-aspect container — `maxHeight` flows it to hug the image height (sidebar preview, so landscape pictures get no
  letterbox), otherwise it fills its parent (`Lightbox`). The sidebar preview rotates by the live *draft* orientation so rotate clicks show instantly,
  before the debounced commit lands; the rotation is not animated (a CSS transform transition takes the visual long way around the 8→1 orientation
  wrap).
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
  after all files. `initial_tags` set in the dialog are passed on the complete body and assigned atomically server-side. Gallery and tags queries are
  invalidated on the first success and again when all uploads settle.

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
- **Number inputs:** native browser spin arrows are stripped globally in `index.css` (they ignore the theme); use `components/ui/number-input.tsx`
  (`NumberInput`) for styled chevron steppers. It calls native `stepUp`/`stepDown` (honours `min`/`max`/`step`) and keeps the standard
  `e.target.value`
  `onChange` API. Steppers auto-hide when `step="any"` (free-form decimals like GPS lat/lng), leaving a plain arrow-less field. shadcn/ui ships no
  number
  primitive ([issue #4385](https://github.com/shadcn-ui/ui/issues/4385)).
