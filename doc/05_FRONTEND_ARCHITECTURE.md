# Frontend Architecture

Documentation of the **implemented** Archypix web frontend (`front/`) for developers and AI agents working on it. For the HTTP contract it consumes,
see [06_API_REFERENCE.md](06_API_REFERENCE.md); for product semantics, [01_GENERAL_SPECIFICATIONS.md](01_GENERAL_SPECIFICATIONS.md).

---

## 1. Goals and constraints

- **Pure static SPA** — no SSR, no build-time data fetching. The `dist/` bundle is served from any CDN with an `index.html` fallback for all routes.
  The architecture spec places the frontend as a "static CDN" peer; SSR would add ceremony with no benefit.
- **Agent-friendly** — all component source (including shadcn/ui primitives) lives in `src/`, not `node_modules`. Every component can be read,
  grepped,
  and edited as a normal project file.
- **Federated by design** — there is no single API base URL. The client resolves, per logged-in user, which backend hosts their identity and talks to
  it directly (see §3).
- **MVP-quality UX** — real loading/empty/error states, dark-first theme, responsive three-pane workspace.

---

## 2. Stack and build

| Concern       | Choice                                                                              |
|---------------|-------------------------------------------------------------------------------------|
| UI / language | React 19 + TypeScript (strict: `noUnusedLocals/Parameters`, `verbatimModuleSyntax`) |
| Bundler       | Vite (`@` → `src` alias in `vite.config.ts` + `tsconfig.app.json`)                  |
| Styling       | Tailwind CSS v4 — CSS-first `@theme` in `src/index.css` (zinc neutral, sky primary) |
| Components    | shadcn/ui in `src/components/ui/` (Radix primitives, copied source)                 |
| Server state  | TanStack Query v5                                                                   |
| Client state  | Zustand                                                                             |
| Routing       | React Router v7                                                                     |
| Forms         | React Hook Form + Zod (`src/lib/schemas.ts`)                                        |
| HTTP          | axios                                                                               |
| Drag & drop   | @dnd-kit (pipeline reordering)                                                      |
| Misc          | blurhash (thumbnail placeholders), sonner (toasts), lucide-react (icons)            |

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
| `/settings`    | `SettingsPage`      | required   | profile + versioning mode                         |
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

| Store          | Shape                                                                                | Persistence (`localStorage`) |
|----------------|--------------------------------------------------------------------------------------|------------------------------|
| `auth.ts`      | `user, accessToken, refreshToken, backendUrl, instance` + setters/`clear`            | `archypix_auth`              |
| `ui.ts`        | `leftSidebarOpen, rightSidebarOpen, rowHeight, tagProvenance` + actions              | `archypix_ui`                |
| `theme.ts`     | `theme: 'dark' \| 'light'` (applies/removes `.light`); `initTheme()` at boot         | `archypix_theme`             |
| `selection.ts` | `selected: string[], anchor` — gallery multi-select (click / ⌘-toggle / shift-range) | none (session only)          |

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

| Domain   | `api/*`                                                                               | `hooks/*`                                                                                   |
|----------|---------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| auth     | `auth.ts` — `login, logout, register, fetchMe`                                        | (imperative; not query-backed)                                                              |
| pictures | `pictures.ts` — `listPictures, getPicture, getPictureUrl, editPicture, getJob`        | `usePictures` (infinite, `thumbnail:'medium'`, page 50), `usePictureEdit.useEditExif`       |
| tags     | `tags.ts` — `listAllTags, listPictureTags, listPictureTagsWithSources, batchEditTags` | `useTags` — `useAllTags, usePictureTags, useBatchEditTags`                                  |
| shares   | `shares.ts` — `list/accept/reject/revoke/createOutgoing`                              | `useShares` — `useIncomingShares, useOutgoingShares, useShareMutations`; `useShareMappings` |
| tagging  | `tagging.ts` — service + rule/segment/mapping CRUD, `reorderServices`                 | `useTaggingServices` — `useTaggingServices, useTaggingService, useTaggingMutations`         |
| settings | `settings.ts` — `getSettings, updateSettings, updateProfile`                          | `useSettings` — `useSettings, useUpdateSettings, useUpdateProfile`                          |

`apiErrorMessage(error)` (in `api/client.ts`) extracts a human string for toasts. `hooks/useDebouncedValue.ts` backs the search box.

**Tag paths** are dot-separated ltree **wire form** (`Photos.Travel.Alps`) on the wire and slash **display form** (`/Photos/Travel/Alps`) in the UI.
Convert via `lib/utils.ts` → `TagPath`: `toDisplay`, `toWire`, `segments`, `leaf`, `isProtected`. Share identities encode `@`→`_AT_`, `.`→`_DOT_`
within a label (e.g. `SharedToMe.alice_AT_ex_DOT_com.Photos`). `SharedToMe` is the reserved **protected** prefix.

---

## 7. Component map (`src/components/`)

**`layout/`** — `AppShell` (chrome), `TopBar` (single unified bar: brand + nav + sidebar toggles + gallery search/filters + row-height slider +
theme +
user; gallery-only controls keyed on `pathname === '/'`), `LeftPanel` (shadcn `Tabs`: Tags / Incoming / Outgoing / Hierarchies, synced to the `panel`
URL param), `ProtectedRoute`, `PagePlaceholder`.

**`photos/`** — `PhotoGrid` (justified flex grid + infinite scroll + selection + renders the Lightbox), `PhotoCard` (`flex-basis`/`flex-grow` from the
picture's aspect ratio + `aspect-ratio` on the cell → uniform row height, no crop), `Blurhash`, `FilterControls` (search + scope + sort + filters,
rendered inside `TopBar`), `RowHeightSlider`, `Lightbox` (full-screen carousel driven by the `view` param; ←/→/Esc; `large` variant), `SelectionPanel`
(right panel; see §8).
**`photos/detail/`** — `Section` (compact foldable section, collapse persisted per id), `ExifEditDialog`.

**`tags/`** — `TagTree` (recursive hierarchy from `useAllTags`; click sets the `tag` filter), `TagPicker` (autocomplete over existing tags +
create-new;
`allowProtected` prop — see §9).

**`tagging/`** — `TaggingPage` composes `SharedMappingSection` (shared-tag-mapping services in a **collapsed-by-default accordion, always first**)
then
`PipelineList` (rule + segmentation services, **@dnd-kit reorder that never includes shared_tag_mapping ids**) of `ServiceCard`s.
`RequiresExcludesEditor`
(gates as a **local draft committed on Save**), `RuleEditor`, `SegmentEditor`, `MappingEditor`, `DeleteServiceDialog` (promote-vs-remove),
`NewServiceMenu`.

**`shares/`** — `IncomingSharesList` (accept / reject(confirm) / view-photos; single local-tag mapping per share via `useShareMappings`),
`OutgoingSharesList` (**grouped by tag**, per-recipient status + confirm-revoke), `CreateShareDialog`, `ShareStatusBadge`.

**`common/`** — `ConfirmDialog` (AlertDialog wrapper gating sensitive actions).

---

## 8. The gallery workspace

`GalleryPage` is a three-pane layout under the unified `TopBar`; each side panel is shown only when its `ui` store toggle is on:

- **Left** (`LeftPanel`): tabbed Tags tree / Incoming shares / Outgoing shares / Hierarchies (placeholder).
- **Center** (`PhotoGrid`): the justified grid; double-click opens the `Lightbox`.
- **Right** (`SelectionPanel`): for a single selection, foldable `Section`s in order — name (+ size/dimensions), **Tags** (with a persisted
  list⇄provenance toggle: provenance fetches `with_sources` and shows per-source badges), **Shared with you** (the `SharedToMe.*` tags), **Shared by
  you** (outgoing shares whose tag covers this picture), **Details**, **EXIF** (with `ExifEditDialog` for owned pictures), **Versions**. For a
  multi-selection: batch tag-add. Tags, provenance sources, and shared-with-you entries are clickable cross-links (see §9).

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
- **Single mapping per share:** `useShareMappings.addMapping` deletes any existing mapping first; the tagging `MappingEditor` hides already-mapped
  shares.
- **Cross-links:** right-panel tag → sets the `tag` filter; a provenance source badge → `/tagging/:source_id` (or `panel=incoming` + `share` highlight
  for an `incoming_share` source); a "Shared with you" tag → `tag` filter + `panel=incoming` + highlights the matching card in `IncomingSharesList`.
- **Search:** the API has **no free-text search**; `q` is a client-side filename filter over already-loaded items (it is still kept in the URL for
  future server-side search). The capture-date range filter has a reserved slot in the Filters menu but is not built yet.
- **Sensitive actions** (revoke / reject / delete) are gated by `ConfirmDialog`.

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
