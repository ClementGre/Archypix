# Shared agent conventions

- **Comments: short and sparse, strict.** ≤1–2 lines, only for non-obvious *why*, or a pointer to the
  doc with full rationale (e.g. `// priority §5.1 — see doc/features/11`). Never multi-line paragraph
  comments re-explaining design/algorithm/what-the-code-does — that belongs in `doc/`, referenced
  with a one-liner.
- Keep docs up to date, at the level of detail already present — no overly specific blow-by-blow of a
  single change.
- Editing the API → update doc/06_API_REFERENCE.md. Completing a task → update doc/99_ROADMAP.md.
- Do not run cargo fmt crate-wide — HEAD is not rustfmt-clean.
- Do not start or stop Docker containers; if the dev Postgres is down, report it instead.
- Always favor refactoring and rebuilding over patching. Code should stay simple with no boilerplate
  from deprecated paradigms.
- Don’t write code duplicate. Factorize code as much as possible. In frontend reuse/repurpose
  existing components or redevelop components instead of duplicating code. Same logic in backend.

# Read-me gates

What to read *before* you act. Layer-specific coding guidelines live inside that layer's architecture
doc (03 §I backend/Rust, 05 §11 frontend), not here.

**Always:**

- Changing *what the system does* (tags, trash, tagging pipeline, hierarchies, federation, sharing),
  or unsure of intended behaviour → read the relevant § of **01_GENERAL_SPECIFICATIONS.md**.
- Touching deployment topology, the resolver, or a cross-service invariant → read
  **02_INFRASTRUCTURE_DESIGN.md** (short; the Invariants section is the point).
- Completing any task → update **99_ROADMAP.md** and the feature's Work-breakdown (one line per task).

**By what you touch:**

- `back/**` or any Rust-workspace file → **read 03_BACKEND_ARCHITECTURE.md §I first** (~1 screen:
  migrations + `cargo sqlx prepare`, Rust conventions, tracing, common mistakes, the back/worker agent
  checklist) *before* writing code. Wider context: 03 §A–H (AppState, pipeline, API conventions,
  federation).
- `worker/**` → read **04_WORKER_ARCHITECTURE.md** (module layout, job loop, claim-token protocol,
  EXIF/thumbnail/video jobs). 03 §I also applies.
- `resolver/**` → read **07_RESOLVER_ARCHITECTURE.md** (factual state of the `archypix-resolver`
  crate). Planned work is feature 23.
- `front/**` → **read 05_FRONTEND_ARCHITECTURE.md §11 first** (no dev server, build-check only).
  Routes/stores/components in 05 §1–10.
- Adding or changing an HTTP endpoint (either side) → read **and update** 06_API_REFERENCE.md.
- Changing a feature's behaviour → read the matching **doc/features/NN_*.md** first (index below),
  then update its Work-breakdown + Documentation-updates sections.

# Finding the right lines

Docs are long; read the section you need, not the file. Get the section index by running:

```bash
cd doc && grep -n '^## ' [0-9]*.md
```

For `06_API_REFERENCE.md`, whose §6 alone is ~1750 lines, go one level deeper:

```bash
grep -n '^### ' doc/06_API_REFERENCE.md
```

Then read from that line. Never cache these numbers anywhere — they drift on every edit; re-run the
grep instead.

# Feature docs (`doc/features/`)

What each one is, since that is not derivable from its headings. Use the grep above (over
`doc/features/*.md`) for their sections.

- **01_better_sharing_support** — per-picture token model, pipeline-driven announce/unannounce,
  ShareBack, loop prevention, transitive sharing.
- **02_pipeline_announcement_robustness** — share state machine, deliver-then-record ordering,
  per-user wake model, backoff.
- **03_recurring_tasks_framework** — first pass at scheduled tasks (superseded in shape by 17).
- **04_better_exif_support** — EXIF write-through model, concurrency, versioning policy, MIME preflight.
- **05_hierarchies** — node-tree `config` JSONB, read resolution, `TagPredicate`, write-back, naming.
- **06_webdav** — WebDAV server, `VirtualFs`, auth, reads/writes, mirror auto-tag, caching.
- **07_security_audit** — what was verified sound, findings, hardening priority.
- **08_webdav_issues** — single note: editing a picture with Preview.
- **09_trash_and_exif_overrides** — soft delete, owner-deletion propagation, recipient EXIF overrides.
- **10_recipient_exif_editing** — per-share `allow_exif_edit`, `local`/`propose` modes, propagation.
- **11_physical_copy_and_dedup** — rescue-copy with provenance, `content_hash`, dedup reconciler,
  boomerang guard.
- **12_observability_tracing** — structured span-correlated logs, OTel export to Jaeger.
- **13_better_rules** — structured JSONB predicate tree (AND/OR/NOT), field coverage, evaluation.
- **14_better_batch_editing** — `PictureSelection`/`PictureFilter`, batch aggregate/write, dry runs.
- **15_qol_improvements** — loose list of front UX items and strange edge cases.
- **16_trace_sampling_and_collector** — sampling + OTel Collector deployment hardening (not built).
- **17_unified_routine_framework** — the routine framework the other mechanisms migrated onto.
- **18_hierarchy_improvements** — `drop` nodes, per-node write-back tri-state, mirror depth/excludes.
- **19_exiftool_metadata_engine** — evaluation of replacing rexiv2 with ExifTool (recommendation).
- **20_calendar_segmentation** — Calendar partition operator, template grammar, unified service config.
- **21_photos_fix_tools** — **superseded** by 29 + 30 (rough stub, kept for the trail).
- **22_storage_quotas** — per-user counters, delta accounting, enforcement points, reconcile.
- **23_resolver_admin_and_runtime_config** — delegation tokens, runtime config, resolver admin,
  registration rules, placement strategies. Pairs with doc/07.
- **24_resolver_admin_frontend** — the frontend half of 23.
- **25_resolver_chore** — resolver router under one `/archypix-resolver/` prefix; frontend bootstrap.
- **26_picture_creator** — owner-vs-creator attribution field, format convention, propagation.
- **27_public_shares** — link-gated pull shares, anonymous contribution, convert. Depends on 26.
- **28_federation_robustness** — read-path failure isolation, outbound HTTP hardening, one typed
  versioned envelope, crash-atomicity, rate limiting.
- **29_query_proximity_and_missing_filter** — presence filters, directed bracketing, proximity sorts.
  Substrate for 30.
- **30_photos_fix_tools** — guided GPS/capture-date fix modes. Depends on 29 + 04 + 09/10 + 14.
- **31_robust_exif_sync** — state-based EXIF sync: `file_exif` snapshot, claim-time target,
  `write_failed`.
- **32_webdav_file_modified_at** — `pictures.file_modified_at`, the bytes-only WebDAV last-modified.
- **33_exif_read_path_and_engine_fallback** — EXIF *read* path: ExifTool dispatch + fallback, observed
  sync states.
- **34_tag_metadata** — decorative `(user_id, tag_path)` side table. Substrate for 35 (spec-only).
- **35_timeline_view** — structured browse over any tag subtree + the gallery filter-param rewrite
  (spec-only; depends on 34).
