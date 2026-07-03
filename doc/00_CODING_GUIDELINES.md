# Documentation Index

Where to look depending on the task. Layer-specific coding guidelines (migrations, Rust
conventions, tracing, frontend conventions) live inside that layer's architecture doc, not here.

## General — read regardless of task

- doc/01_GENERAL_SPECIFICATIONS.md: product/domain spec — tags, trash, tagging pipeline, hierarchies, federation, sharing. What the system does.
- doc/02_INFRASTRUCTURE_DESIGN.md: deployment topology — resolver, backend, workers, MinIO, frontend, and their invariants.
- doc/99_ROADMAP_MVP.md: current status. Update when you complete a task.

## Specific — pick by what you're touching

- doc/03_BACKEND_ARCHITECTURE.md: `back/` layered architecture, `AppState`, pipeline internals, API conventions, federation flows. **§I has
  backend/Rust guidelines** (migrations, Rust conventions, tracing, common mistakes). Read for any `back/` or Rust-workspace-wide work.
- doc/04_WORKER_ARCHITECTURE.md: `worker/` module layout, job loop, claim-token protocol, EXIF/thumbnail/video jobs, shared `archypix-common` types.
  Read for `worker/` work (also applies doc/03_BACKEND_ARCHITECTURE.md §I).
- doc/05_FRONTEND_ARCHITECTURE.md: `front/` stack, routes, stores, data layer, components, gotchas. **§11 has frontend guidelines** (no dev server,
  build-check only). Read for any `front/` work.
- doc/06_API_REFERENCE.md: full HTTP endpoint catalog. Read/update when consuming or editing an API endpoint, from either layer.
- doc/features/NN_*.md: per-feature design docs, referenced from the architecture docs above. Read the matching one before changing that feature's
  behavior.

## Shared agent conventions

- **Comments: short and sparse, strict.** ≤1–2 lines, only for non-obvious *why*, or a pointer to the doc with full rationale (e.g.
  `// priority §5.1 — see doc/features/11`). Never multi-line paragraph comments re-explaining design/algorithm/what-the-code-does — that belongs in
  `doc/`, referenced with a one-liner.
- Keep docs up to date, at the level of detail already present — no overly specific blow-by-blow of a single change.
- Editing the API → update doc/06_API_REFERENCE.md. Completing a task → update doc/99_ROADMAP_MVP.md.

Layer-specific rules (migrations, Rust idioms, tracing, frontend dev-server ban, …): doc/03_BACKEND_ARCHITECTURE.md §I,
doc/05_FRONTEND_ARCHITECTURE.md §11.
