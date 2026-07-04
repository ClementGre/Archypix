# Documentation Index

Where to look, by task. Two parts: **gates** (what to read *before* you act) and a **section index**
(jump straight to the right lines). Layer-specific coding guidelines live inside that layer's
architecture doc (03 §I backend/Rust, 05 §11 frontend), not here.

## Read-me gates

**Always:**

- Changing *what the system does* (tags, trash, tagging pipeline, hierarchies, federation, sharing),
  or unsure of intended behaviour → read the relevant § of **01_GENERAL_SPECIFICATIONS.md** (see index).
- Touching deployment topology, the resolver, or a cross-service invariant → read
  **02_INFRASTRUCTURE_DESIGN.md** (Invariants at L62).
- Completing any task → update **99_ROADMAP_MVP.md** and the feature's Work-breakdown (no more than one line per task).

**By what you touch:**

- `back/**` or any Rust-workspace file → **read 03_BACKEND_ARCHITECTURE.md §I first** (L374, ~1
  screen: migrations + `cargo sqlx prepare`, Rust conventions, tracing, common mistakes, the
  back/worker agent checklist) *before* writing code. Wider context: 03 §A–H (AppState, pipeline, API
  conventions, federation).
- `worker/**` → read **04_WORKER_ARCHITECTURE.md** (module layout, job loop, claim-token protocol,
  EXIF/thumbnail/video jobs). 03 §I also applies.
- `front/**` → **read 05_FRONTEND_ARCHITECTURE.md §11 first** (L541 — no dev server, build-check
  only). Routes/stores/components in 05 §1–10.
- Adding or changing an HTTP endpoint (either side) → read **and update** 06_API_REFERENCE.md.
- Changing a feature's behaviour → read the matching **doc/features/NN_*.md** first (index below),
  then update its Work-breakdown + Documentation-updates sections.

## Shared agent conventions

- **Comments: short and sparse, strict.** ≤1–2 lines, only for non-obvious *why*, or a pointer to the
  doc with full rationale (e.g. `// priority §5.1 — see doc/features/11`). Never multi-line paragraph
  comments re-explaining design/algorithm/what-the-code-does — that belongs in `doc/`, referenced
  with a one-liner.
- Keep docs up to date, at the level of detail already present — no overly specific blow-by-blow of a
  single change.
- Editing the API → update doc/06_API_REFERENCE.md. Completing a task → update doc/99_ROADMAP_MVP.md.

## Section index (file › section › line)

Jump targets — `L<n>` is the heading's line. Line numbers drift on edit; treat as approximate.

### Core docs

- **01_GENERAL_SPECIFICATIONS.md** — 1 Core Model: Tags (L3) · 2 Deletion & Trash (L26) ·
  3 TaggingServices (L55) · 4 Hierarchies / bidirectional WebDAV (L158) · 5 Federation (L237) ·
  6 Sharing (L259) · 7 Important Edge Cases (L365)
- **02_INFRASTRUCTURE_DESIGN.md** — single short doc (68L); deployment topology, **Invariants** (L62)
- **03_BACKEND_ARCHITECTURE.md** — A) Technology stack (L3) · B) Layered architecture (L9) ·
  C) Module layout (L28) · D) AppState (L107) · E) Tagging pipeline (L123) · F) API Conventions (L254) ·
  G) Key Flows (L301) · H) Routine framework (L349) · I) Coding guidelines (L374)
- **04_WORKER_ARCHITECTURE.md** — Module layout (L6) · Claim-token protocol (L59) · Job loop (L68) ·
  Error policy (L91) · EXIF edit write-through (L98) · Shared types `archypix-common` (L108)
- **05_FRONTEND_ARCHITECTURE.md** — 1 Goals & constraints (L8) · 2 Stack & build (L22) · 3 Connection
  model / federated (L44) · 4 Routes (L74) · 5 State management (L97) · 6 Data layer (L136) ·
  7 Component map (L160) · 8 Gallery workspace (L354) · 9 Key behaviours & gotchas (L391) ·
  10 Conventions (L528) · **11 Coding guidelines / agents (L541)**
- **06_API_REFERENCE.md** — 1 Route Groups (L7) · 2 Authentication (L23) · 3 Wire Format Conventions
  (L45) · 4 Auth Endpoints (L60) · 5 Public Endpoints (L153) · 6 Authenticated User Endpoints (L212) ·
  7 Admin Endpoints (L1639) · 8 Federation & Worker (L2036) · 9 WebFinger (L2074) · 10 Shared Type
  Reference (L2109) · 11 Key Frontend Behaviours (L2167)
- **99_ROADMAP_MVP.md** — Completed (L3) · To-do for the MVP (L14) · To-do for v1.0 (L52)

### Feature docs (`doc/features/`)

- **01_better_sharing_support.md** — 1 Overview (L3) · 2 Key Design Decisions (L22) · 3 DB Schema
  Changes (L61) · 4 Domain Type Changes (L117) · 5 Per-Picture Token Model (L143) · 6 Pipeline
  Extension (L196) · 7 New Flows (L350) · 8 Extended `cleanup_incoming_share` (L468) · 9 SharedToMe
  Prefix Protection (L510) · 10 Modified/New Services (L539) · 11 Modified/New Repositories (L570) ·
  12 Modified Federation Protocol (L642) · 13 Modified/New API Endpoints (L673) · 14 TaskQueue
  Extension (L728) · 15 Configuration (L793) · 16 Edge Cases (L805) · 17 Test Scenarios (L848)
- **02_pipeline_announcement_robustness.md** — 1 Problems with current path (L11) · 2 Principles (L37) ·
  3 Share state machine (L59) · 4 Deliver-then-record ordering (L96) · 5 Same-backend routing fix
  (L130) · 6 Deleted pictures stay announced (L144) · 7 Per-user wake model (L162) · 8 Backoff (L255) ·
  9 Migrations (L276) · 10 Implementation status (L285)
- **03_recurring_tasks_framework.md** — 1 Overview (L8) · 2 Design (L38) · 3 The three tasks (L158) ·
  4 Wiring in `main.rs` (L256) · 5 Config (L300) · 6 Testing (L315) · 7 Doc updates (L332) · 8 Work
  breakdown (L349)
- **04_better_exif_support.md** — 1 Overview & goals (L3) · 2 Decisions (L24) · 3 Current state & bugs
  (L46) · 4 Write-through model (L74) · 5 Concurrency (L109) · 6 Schema changes (L139) · 7 Domain & API
  (L159) · 8 MIME preflight (L237) · 9 Versioning policy (L245) · 10 Federation (L275) · 11 Edge cases
  (L307) · 12 Doc updates (L329) · 13 Work breakdown (L343)
- **05_hierarchies.md** — 1 Overview & goals (L3) · 2 Decisions (L32) · 3 Conceptual model (L66) ·
  4 `config` JSONB schema (L85) · 5 Read resolution (L197) · 6 `TagPredicate` & `list_pictures` (L301) ·
  7 Write-back model (L351) · 8 Naming strategy (L421) · 9 API (L437) · 10 DB changes (L485) ·
  11 Validation (L497) · 12 Module layout (L514) · 13 Out of scope (L530) · 14 Edge cases (L548) ·
  15 Testing (L561) · 16 Doc updates (L572)
- **06_webdav.md** — 1 Overview & goals (L3) · 2 Decisions (L33) · 3 Authentication (L79) · 4 Server,
  routing & locking (L134) · 5 `VirtualFs` abstraction (L153) · 6 Reads (L192) · 7 Writes (L208) ·
  8 Identity resolution (L262) · 9 Mirror auto-tag & MKCOL (L284) · 10 Case-sensitivity (L305) ·
  11 Dotfiles (L322) · 12 Other edge cases (L330) · 13 Caching (L342) · 14 DB changes (L350) ·
  15 Config (L362) · 16 Module layout (L368) · 17 API token mgmt (L388) · 18 Testing (L403) · 19 Out of
  scope (L415) · 20 Doc updates (L424) · 21 Implementation status & MVP deviations (L436)
- **07_security_audit.md** — 1 What was verified SOUND (L16) · 2 Findings / hardening (L91) ·
  3 Suggested priority (L248)
- **08_webdav_issues.md** — single note (44L): editing a picture with Preview (L3)
- **09_trash_and_exif_overrides.md** — 1 Overview & goals (L3) · 2 The invariant (L27) · 3 Decisions
  (L46) · 4 Schema changes / consolidated 09+10+11 (L71) · 5 Trash & restore (L136) · 6 EXIF overrides
  (L169) · 7 Coverage & announcement (L201) · 8 Federation (L230) · 9 Edge cases (L244) · 10 Future:
  physical copy (L263) · 11 Doc updates (L273) · 12 Work breakdown (L282)
- **10_recipient_exif_editing.md** — 1 Overview & goals (L3) · 2 Decisions (L24) · 3 Permission model
  (L40) · 4 API (L54) · 5 Propagation flow (L116) · 6 Edge cases (L137) · 7 Doc updates (L164) · 8 Work
  breakdown (L173)
- **11_physical_copy_and_dedup.md** — 1 Overview & goals (L3) · 2 Decisions (L22) · 3 Copy endpoint
  (L44) · 4 `content_hash` (L63) · 5 Dedup reconciler (L80) · 6 Boomerang / loop prevention (L182) ·
  7 Edge cases (L196) · 8 WebDAV write-back open question (L218) · 9 Doc updates (L228) · 10 Work
  breakdown (L238)
- **12_observability_tracing.md** — 1 Overview & goals (L3) · 2 Decisions (L60) · 3 Structured,
  span-correlated logs (L81) · 4 OTel export to Jaeger (L263) · 5 Configuration (L450) · 6 Jaeger
  deployment (L460) · 7 Files touched (L478) · 8 Testing (L509) · 9 Roadmap (L523)
- **13_better_rules.md** — 1 Motivation (L3) · 2 Predicate model (L19) · 3 Rule structure (L127) ·
  4 Schema change (L144) · 5 `PipelineInput` additions (L165) · 6 Validation (L187) · 7 Evaluation
  (L204) · 8 API changes (L220) · 9 What this does NOT change (L230)
- **14_better_batch_editing.md** — 1 Motivation (L3) · 2 Selection descriptor (L24) · 3 Homogenized
  picture filter (L73) · 4 Batch read / aggregation (L100) · 5 Deferred EXIF jobs (L209) · 6 Batch
  write surface (L236) · 7 Frontend (L307) · 8 Out of scope (L322)
- **15_qol_improvements.md** — Front UX (L3) · Frontend UX affecting back (L30) · Strange edge cases
  (L44)
- **16_trace_sampling_and_collector.md** — 1 Overview & goals (L3) · 2 Why these matter (L22) · 3 Plan
  (L77) · 4 Decisions (L110) · 5 Configuration (L120)
- **17_unified_routine_framework.md** — 1 Overview (L3) · 2 Design (L47) · 3 Migrating each mechanism
  (L190) · 4 Durability caveat (L224) · 5 Config (L234) · 6 Testing (L240) · 7 Migration order (L254) ·
  8 Doc updates (L266) · 9 Open questions (L277)
- **18_hierarchy_improvements.md** — 0 Implementation status (L3) · 1 Overview & goals (L19) ·
  2 Decisions (L38) · 3 Config schema changes (L69) · 4 Drop directory node (L122) · 5 Per-node
  write-back (L141) · 6 Write-back on untagged queries (L187) · 7 Mirror depth limit & foreign excludes
  (L202) · 8 Implementation (L244) · 9 Edge cases (L296) · 10 Testing (L311) · 11 Doc updates (L324) ·
  11(bis) Other things to make sure (L340)
- **19_exiftool_metadata_engine.md** — 1 Motivation (L7) · 2 What it would/would not replace (L25) ·
  3 Why it could be better (L42) · 4 The cost (L52) · 5 Rough migration shape (L68) · 6 Recommendation
  (L86)
- **20_calendar_segmentation.md** — 1 Motivation (L3) · 2 Model (L24) · 3 `SegmentationConfig` schema
  (L58) · 4 Template grammar (L108) · 5 Placeholder config `parts` (L167) · 6 `offset` boundary
  shifting (L213) · 7 Resolution algorithm (L239) · 8 Worked example (L258) · 9 Validation (L292) ·
  10 Storage: unified service config (L309) · 11 Migration of existing data (L393) · 12 Evaluation,
  API, frontend (L422) · 13 Out of scope: clustering (L478) · 14 What this does NOT change (L495)
- **21_photos_fix_tools.md** — GPS fix (L5) · Capture date fix (L12)
- **22_storage_quotas.md** — 1 Overview & goals (L3) · 2 Decisions (L20) · 3 What counts (L36) ·
  4 Schema changes (L55) · 5 Delta accounting & upload race (L98) · 6 Enforcement points (L135) ·
  7 Reconcile routine (L153) · 8 API (L163) · 9 Resolver seed (L210) · 10 Config (L223) ·
  11 Frontend (L235) · 12 Edge cases (L247) · 13 Testing (L270) · 14 Doc updates (L282) ·
  15 Work breakdown (L293)
