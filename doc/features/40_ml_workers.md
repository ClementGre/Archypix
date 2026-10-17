# Feature 40: ML analysis & grouping models

## 0. Status & how to use this document

**Spec, ready for implementation.** Built over 14 design rounds; this is a clean rewrite with every
decision taken. §19 cuts the work into **12 phases**. Each phase is sized for one agent, leaves the
tree compiling with its tests passing, and is followed where relevant by a frontend phase that
exercises it. The phases run one by one, in order.

Rules for every phase agent:
- Read `AGENTS.md` first, then 03 §I (back/worker) or 05 §11 (front), then this document's §1–§3 and
  your phase in §19. Only then read the sections your phase cites.
- Never start or stop Docker containers. If the dev Postgres is down, stop and report it.
- Never run `cargo fmt` workspace-wide. Format only the files you touched.
- After a migration: `cargo sqlx migrate run && cargo sqlx prepare -- --tests` from `back/`, then
  regenerate `back/migrations/schema.sql` (03 §I).
- Keep comments short (AGENTS.md). Put rationale here, not in code.
- Update the docs your phase lists, `doc/99_ROADMAP.md`, and this document's §19 checklist (tick your
  phase, and note any deviation under it).
- Finish with one commit on `main` (no push).

The pgvector Postgres image is already in place (§6.1).

---

## 1. Goals & non-goals

Give Archypix "smart library" features **without breaking the tag-first model**. ML never becomes a
parallel organisation system. What it concludes surfaces as **tags** (through pipeline services) or
as **query primitives** (rule predicates, search, sorts), so hierarchies, WebDAV, sharing and rules
keep working unchanged.

Delivered by this feature:
- **People:** faces detected and grouped into people, named by the user, projected as tags
  (`People.Alice`).
- **Concepts:** pictures classified into user-defined, learnable concepts (`Kinds.Screenshot`,
  `Kinds.Document.Receipt`).
- **Semantic search:** text query ("dog on a beach", any language) and "more like this".
- **Timeline:** zones (home, work), events (trips) and series (recurring places), projected as tags
  (`Trips.2024.Rome_2024_05`, `Zones.Home`).
- **Federation:** analyses travel to share recipients; a person can carry an identity hint.

**Non-goals:** cloud inference APIs, generative edits, pets as identities, training anything beyond
a per-concept linear probe, GPU images, model-set upgrades (§20).

---

## 2. Vocabulary

| Term | Meaning |
|:-----|:--------|
| **Analysis** | Per-picture ML output: one image embedding plus detected faces (box, landmarks, score, embedding). Content-keyed and shared by every row, user and instance showing the same bytes. |
| **Model** | A user-level grouping model of one **kind**: `people` (exactly one per user), `concepts` or `timeline` (any number). Holds **groups**, learning settings, **scope gates** and corrections. |
| **Group** | One person, one concept, or one zone / event / series. Stable id, plus a **slug** (its tag label). |
| **Grouping service (tree)** | A tagging service of type `grouping` that projects **one** model into **one** tag tree under its **root**, with its own gates, selection and layout. A model may have any number of trees. |
| **Primary** | The model's **at most one** editable tree. Manual tags under its root are the model's decisions, its grouping tags are removable as corrections, and the face API writes there. Every other tree is a **read-only view**. |
| **Decision** | User truth: a **manual tag** under the primary root (positive), a **rejection** (negative, from a removal), a **face pin** (face-level pick), a **face flag** (*not a person*). |
| **Derived member** | A solver result: picture (and face, for people) → group, with how it was placed (`pin` / `manual` / `auto`). Never user truth; cached only to diff and list. |
| **Scope gates** | The model's `requires` / `excludes`: what it learns from and may tag at all. |
| **Tree gates** | A service's ordinary `requires` / `excludes`: which in-scope pictures get *that* tree's tags. |

---

## 3. Architecture & invariants

```
 worker ml_analyze ──► ml_analyses / ml_faces  (content-keyed, federated by pull)
                            │
                 model routines (per model)  ── read: analyses, decisions, stored tags
                            │   write: model state (prototypes, probes, partitions), derived cache
                            ▼
 pipeline (per user) ── per picture: solve each needed model (pure fn) ──► grouping trees ► tags
                                                                     └──► rule predicates
```

**Invariants**
1. **The pipeline never runs image inference and never writes model state.** It only reads the
   models and writes tags. (A text prompt missing from the embedding cache is embedded on the spot,
   §4.4.)
2. **Every membership is derived** from decisions + model state by pure functions
   (`domain::grouping::{people,concepts,timeline}::solve`). Nothing user-made is overwritten, and a
   derived row can always be recomputed.
3. **Manual tags are never deleted by a grouping service.** `prune_shadowed_manual_tags` is not
   offered for `grouping`.
4. **Only the primary tree's grouping tags are removable** (as rejections). Rule, segmentation,
   share and non-primary grouping tags stay locked. The `unmatched` bucket is never removable.
5. **Subtags are free.** Below a group's path, other services' tags are ignored by the model, and a
   manual subtag asserts its group and is kept.
6. **Grouping roots never overlap** (any two grouping services, validated on save), so every manual
   tag belongs to at most one model.
7. **Invalidation is intrinsic to the write** (03 §E): recording an analysis, writing a rejection,
   pin or flag, and a routine flipping a derived member all re-NULL `last_pipeline_run_at` on the
   affected rows.
8. **Pipeline ↔ routine handshake.** When the pipeline's solution for a picture differs from the
   derived cache, it wakes that model's routine (debounced). The routine recomputes, rewrites the
   cache, and invalidates only flipped pictures. This converges.

---

## 4. ML stack

### 4.1 The canonical model set

Federated analyses must be comparable, so the model set is a **protocol constant**, not an admin
choice: `MODEL_SET = "2026.1"` in `archypix-ml`. It pins every file (URL at a fixed Hugging Face
revision, sha256, size), the preprocessing, and the thresholds (§4.3).

| Role            | Model                                         | Licence    | Output                       |
|:----------------|:----------------------------------------------|:-----------|:-----------------------------|
| Image embedding | **SigLIP 2** base patch16-256, vision tower   | Apache-2.0 | 768-d, L2-normalised         |
| Text embedding  | SigLIP 2 base patch16-256, text tower + Gemma tokenizer (`tokenizers`) | Apache-2.0 | 768-d, multilingual |
| Face detection  | **YuNet** 2023mar (OpenCV zoo)                | MIT        | boxes + 5 landmarks + score  |
| Face embedding  | **AuraFace v1** (fal, `glintr100.onnx`)       | Apache-2.0 | 512-d, ArcFace-style         |

- Use existing ONNX exports when they exist; otherwise export once with `optimum` and host the
  files at a pinned revision. Either way, record URL, sha256 and size in the registry.
- **Non-commercial weights are excluded.** Archypix is AGPL and may sell hosted storage. InsightFace
  `buffalo_l` / SCRFD / their ArcFace packs are non-commercial.
- **No instance language setting.** Image embeddings are language-agnostic, and the text tower is
  multilingual.

### 4.2 Libraries

| Need                     | Crate                                                    | Where          |
|:-------------------------|:---------------------------------------------------------|:---------------|
| ONNX inference           | `ort` 2.x (CPU EP; prebuilt runtime via `download-binaries`) | `archypix-ml` |
| Tokenizer                | `tokenizers`                                             | `archypix-ml`  |
| Decode / resize / warp   | `image`, `fast_image_resize` (5-point similarity warp hand-rolled) | `archypix-ml` |
| Vectors in Postgres      | `pgvector` (features `sqlx`, `halfvec`)                  | `back`         |
| Assignment               | `pathfinding::kuhn_munkres` (similarities scaled to integers) | `back` (domain) |
| Clustering               | `petal-clustering` (HDBSCAN)                             | `back`         |
| Reverse geocoding        | `kiddo` k-d tree over GeoNames `cities500` + admin1/2 (CC-BY 4.0, attributed in the UI) | `back` (phase 11) |

Pin exact crate versions when adding them. Check on the crate page that each is at least two weeks
old.

### 4.3 Thresholds

They live in `archypix_ml::thresholds`, versioned with the model set. Names and starting values:

| Constant              | Meaning                                                    | Start |
|:----------------------|:-----------------------------------------------------------|:------|
| `FACE_MIN_SCORE`      | YuNet score to keep a detection                            | 0.60  |
| `FACE_ELIGIBLE_SCORE` | score for an **eligible** face (may be auto-assigned)      | 0.80  |
| `FACE_ELIGIBLE_PX`    | min box short side, in analysed-image pixels               | 40    |
| `SIM_AUTO`            | cosine to a prototype for automatic placement              | calibrate |
| `SIM_PLAUSIBLE`       | looser floor for placing an asserted (manually tagged) person | calibrate |
| `SIM_SAME`            | bootstrap: every kept face similar to the others           | calibrate |
| `SIM_MERGE`           | auto-merge an unnamed cluster into a named person          | calibrate |
| `MARGIN`              | "clearly better than the runner-up"                        | 0.05  |
| `CONCEPT_DEFAULT_THRESHOLD` | zero-shot sigmoid threshold                          | calibrate |

Phase 6 calibrates the `SIM_*` values with an `#[ignore]` test on a public labelled face-pair set
(e.g. LFW pairs: false-accept rate ≤ 1 % at `SIM_AUTO`). Phase 3 calibrates the concept threshold
on a handful of labelled pictures. Both record their results in §19.

### 4.4 The `archypix-ml` crate

A new workspace crate used by the worker (vision) and the backend (text):

```rust
pub const MODEL_SET: &str;
pub mod registry;      // ModelFile { name, url, sha256, size }; ensure_files(dir) -> Result<Paths>
pub mod preprocess;    // decode, orientation (EXIF 1–8), resize/normalise, 5-point face alignment
pub mod thresholds;
pub trait ImageEmbedder: Send + Sync { fn embed(&self, rgb: &RgbImage) -> Result<Vec<f32>>; }
pub trait FaceDetector:  Send + Sync { fn detect(&self, rgb: &RgbImage) -> Result<Vec<Detection>>; }
pub trait FaceEmbedder:  Send + Sync { fn embed(&self, aligned_112: &RgbImage) -> Result<Vec<f32>>; }
pub trait TextEmbedder:  Send + Sync { fn embed(&self, text: &str) -> Result<Vec<f32>>; }
pub mod onnx;          // cfg(feature = "onnx"): the real engines
pub mod stub;          // deterministic engines (vectors derived from a hash of the input), for tests
pub fn analyse(engines: &Engines, image: &RgbImage, orientation: u8) -> Result<Analysis>;
```

- `analyse` rotates to upright, embeds, detects, aligns, and embeds faces. It maps boxes and
  landmarks back to the **raw-pixel frame, normalised to [0, 1]**, and sets `eligible` per §4.3.
- `onnx` is a cargo feature: on in the worker and back release builds, off in unit tests. The stub
  engines make every downstream test deterministic and model-free.
- Models download on first use into `ML_MODELS_DIR` (checksum-verified, atomic rename), or are
  pre-placed there for air-gapped installs.
- The worker needs the vision files, the backend only the text tower and tokenizer.

---

## 5. Analysis

### 5.1 Keys and statuses

- `pictures.ml_key` is a **generated column**:
  `coalesce(content_hash, file_hash, 'pic:' || id) || ':' || coalesce(orientation, 1)`.
  - Content-keyed: copies, dedup siblings and received duplicates share one analysis.
  - Orientation-aware: an orientation edit (EXIF-only, so `content_hash` is unchanged) yields a new
    key and a re-analysis, because faces are detected upright.
- `pictures.ml_status ∈ {pending, analyzed, unsupported, failed}`, default `pending`. A picture is
  analysed iff an `ml_analyses` row exists for `(ml_key, MODEL_SET)`.

### 5.2 The `ml_analyze` job

- Replaces the three empty stubs (`ml_style`, `ml_people`, `ml_group_location`), which encode no
  design. The `job_type` enum is rebuilt to `{gen_thumbnail, edit_picture, ml_analyze}`.
- `JobConfig::MlAnalyze { picture_id, ml_key, orientation }`. The key and orientation are **bound at
  claim** (like feature 31's EXIF target) and persisted on the job.
- **Claim** returns a presigned GET for the **`large` thumbnail** (1000 px high, raw-pixel):
  - owned: the local large bucket;
  - received: `services::pictures::presign_variant_for_picture` (proxies to the owner).
- **Worker:** `jobs/ml.rs` downloads, decodes, calls `archypix_ml::analyse`, and returns
  `JobProduct::Ml(MlAnalysis { model_set, embedding, faces: [{ bbox, landmarks, score, eligible,
  embedding }] })`.
  - A missing or undecodable thumbnail → `failed` (permanent).
  - A network or model-load fault → `retry`.
- **Recording** (`respond_job`, new branch, `services::ml::record_analysis`):
  - validate `model_set`, dimensions and finite values;
  - insert the analysis for the bound key, or keep the existing row if a concurrent job won;
  - set `ml_status = analyzed` and re-NULL `last_pipeline_run_at` on **every row with that
    `ml_key`**;
  - after commit, wake the pipeline (debounced) and the model routines of every affected user.
- **Priority:** `jobs.priority smallint` (0 for `gen_thumbnail` / `edit_picture`, 10 for
  `ml_analyze`). `claim_next` orders by `priority, created_at`, and the partial pending index
  follows.
- **Worker throughput:** `MAX_CONCURRENT_ML_JOBS` (default 1). While the ML slots are full, the claim
  loop drops `ml_analyze` from `ClaimQuery.types`.
- **Worker capability:** a worker whose engines fail to load (no models, no network) never claims
  `ml_analyze`, and its `/health` reports `ml: false`.

### 5.3 The `ml_drain` routine

A `()`-keyed routine (pattern: `routines::exif_drain`), triggered after thumbnail completions and
picture ingests, with an interval backstop (`ml_drain_interval_secs`, 60). Each pass, in bounded
batches (`ml_drain_batch`, 500):
1. **Reuse:** a `pending` row whose `ml_key` already has an analysis is set to `analyzed`, with its
   pipeline invalidated.
2. **Unsupported:** an owned row whose thumbnail job finished without thumbnails is set to
   `unsupported`.
3. **Stale:** an `analyzed` row with no analysis for its current `ml_key` (an orientation or content
   change) is set back to `pending`.
4. **Enqueue:** one `ml_analyze` job per distinct pending `ml_key`, interleaving users (round-robin),
   with the idempotency key `ml:{ml_key}:{MODEL_SET}`.
   - Owned rows need `thumbnails_generated_at`.
   - Received rows are enqueued directly (phase 8 inserts the owner pull before this step).
5. **Trash:** trashed rows are skipped. They keep any analysis they have.

With `ml_enabled = false` the drain does nothing.

### 5.4 Garbage collection

A daily `ml_gc` sweep deletes analyses whose `ml_key` no picture row carries. Faces cascade. Pins on
those faces are remapped first (§7.1, *re-analysis*), so the sweep never drops a remappable pin.

---

## 6. Storage

```sql
-- content-keyed analyses (phase 2)
ml_analyses (id uuid PK, ml_key text, model_set text, origin text CHECK (origin IN ('local','remote')),
             embedding halfvec(768), analyzed_at timestamp, UNIQUE (ml_key, model_set))
ml_faces    (id uuid PK, analysis_id uuid REFERENCES ml_analyses ON DELETE CASCADE, face_index smallint,
             bbox real[4], landmarks real[10], score real, eligible bool, embedding halfvec(512))
            -- HNSW index on embedding (halfvec_cosine_ops)
pictures    + ml_key text GENERATED ALWAYS AS (...) STORED, + ml_status ml_status NOT NULL DEFAULT 'pending',
            + index (ml_key)
jobs        + priority smallint NOT NULL DEFAULT 0
ml_text_embeddings (model_set, text, embedding halfvec(768), PRIMARY KEY (model_set, text))   -- prompt cache

-- models (phase 3)
grouping_models (id, user_id, kind grouping_kind, name, requires ltree[], excludes ltree[],
                 settings jsonb, created_at, updated_at)
                -- partial unique (user_id) WHERE kind = 'people'
model_groups    (id, model_id REFERENCES grouping_models ON DELETE CASCADE, slug text, parent_id NULL,
                 hidden bool, merged_into NULL, member_count int, cover_picture_id NULL,
                 cover_focus real[4] NULL, data jsonb, created_at, updated_at,
                 UNIQUE NULLS NOT DISTINCT (model_id, parent_id, slug))
                -- data: people {identity, share_name, category, prototypes}, concepts {prompts,
                --       probe, bias, threshold, exclusive}, timeline {kind, series_id, place,
                --       starts_at, ends_at}
rejections      (model_id, picture_id, group_id, created_at, PRIMARY KEY (model_id, picture_id, group_id))
derived_members (model_id, picture_id, group_id, face_id NULL, how text, score real,
                 PRIMARY KEY (model_id, picture_id, group_id))
tagging_services.service_type + 'grouping';  tag_source + 'grouping'

-- people (phase 6)
face_pins   (model_id, face_id REFERENCES ml_faces ON DELETE CASCADE, person_id NULL, state text,  -- 'pinned' | 'hold'
             PRIMARY KEY (model_id, face_id))
face_flags  (user_id, face_id, not_a_person bool, PRIMARY KEY (user_id, face_id))
-- prototypes live in model_groups.data as base64 halfvec arrays (≤ 4 per person)

-- covers (phase 4)
tag_metadata + cover_focus real[4] NULL
```

- **One generic group table** (`model_groups`) with a kind-specific `data` JSONB, rather than three.
  Every model kind needs id, slug, hierarchy (concept tree, series → event), hidden, merge, count and
  cover. Kind-specific fields are small and read whole.
- **Display names** come from `tag_metadata.display_name` (feature 34). Slugs obey the tag charset
  (`Elodie_Dupont`).
- **`ml_faces.id` is the face identity** for pins and flags. Pins and flags key on face ids and are
  remapped by IoU when an analysis is replaced.

### 6.1 Postgres with pgvector

The compose files use `pgvector/pgvector:0.8.2-pg18-trixie` (official `postgres:18-trixie` + the PGDG
`postgresql-18-pgvector` package).
- **From `postgres:18[-trixie]`:** swap the image and restart.
- **From `postgres:18-alpine`:** run `REINDEX DATABASE <db>;` once per database (musl → glibc
  collation).
- **The extension:** the phase-2 migration runs `CREATE EXTENSION IF NOT EXISTS vector`. If the
  Archypix role may not create extensions (a shared server), an admin runs it once per backend
  database. The resolver database does not need it.

---

## 7. Models

Common to every kind:
- **Creation:**
  - The **people model** is created on first use (People page, first people service, or face API).
  - **Concepts and timeline models** are created by the user, or by "new model" while creating a
    grouping service.
- **Scope gates** (`requires` / `excludes` over tags):
  - **For learning,** the routine evaluates them on **stored** tags.
  - **For projection,** a service evaluates them at its own pipeline position, on the gating tags
    accumulated so far (manual + incoming_share + upstream outputs), as for any gate. A scope that
    names another service's tag therefore needs that service positioned earlier.
  - A scope gate may not reference a path under one of the model's own trees (a cycle, checked on
    save).
- **The routine** is per model, serial, debounced, keyed by model id. It is woken by recorded
  analyses, the handshake (§3 inv. 8), decision writes and settings changes. Each run:
  1. refresh model state;
  2. re-solve the pictures that may have changed;
  3. diff against `derived_members`;
  4. rewrite flipped rows, update `member_count` and the derived cover;
  5. invalidate the flipped pictures and wake the pipeline.
- **Seeding:** a manual tag under the primary root whose group label has no group gets a group with
  that slug.
- **Deleting a model** deletes its trees (each promoting or removing its tags, as today) and its
  state, after confirmation. Deleting a tree leaves the model. If it was the primary, the model reads
  no tag decisions until another tree is made primary.

### 7.1 People

**Faces and eligibility.** Faces come from the analysis. A face is **eligible** per §4.3 and not
flagged *not a person*. Ineligible faces are never auto-assigned, but may be pinned.

**Prototypes.** Each person keeps ≤ 4 prototypes, k-medoids over its **anchored** faces:
- pinned faces;
- faces placed from a manual tag **with certainty** (a forced or clearly-best placement, §9.2);
- automatic placements above `SIM_AUTO + MARGIN`.

Weaker automatic faces never feed prototypes, which prevents drift.

**Discovery.**
- HDBSCAN (`min_cluster_size = 3`) over the **pool** (eligible, unplaced, unflagged, unheld faces)
  proposes new unnamed people, slugged `Person_N`. `N` is a per-model counter, never reused.
- It runs when the user's ML backlog is drained, else at most hourly.

**Bootstrap — a new person's first face from group photos.** It applies when person P has no
prototype and ≥ 3 pictures where P is manually tagged but unplaced:
1. For each candidate face *f* (free or automatically placed) in those pictures, score(*f*) is the
   sum over the other pictures of *f*'s best similarity to their candidates.
2. Each picture keeps its best-scoring face.
3. Accept only if every kept face has ≥ `SIM_SAME` to all the others **and** beats its picture's
   runner-up by `MARGIN`.

The kept faces become P's first prototypes. This is a pure function, unit-tested alone. When it is
not conclusive, nothing happens.

**Auto-merge.** An unnamed person whose prototypes come within `SIM_MERGE` of a **named** person is
merged into it. Two named people are only ever *suggested*.

**Incremental routine.** The model re-solves only pictures that may have changed:
- new analyses;
- pictures whose decisions changed;
- pictures with faces within `SIM_PLAUSIBLE` of a **changed prototype**, found by pgvector KNN on
  `ml_faces` restricted to the model's in-scope pictures;
- pictures carrying a manual tag of a person whose prototypes changed (face-unknown retries).

A full re-solve happens only on a threshold or settings change.

**Re-analysis** (orientation or content change). Pins move to the new analysis's face with IoU ≥ 0.5
in the raw frame; otherwise they are dropped (the manual tag stays). Flags are remapped the same way.

**Categories** are model settings: `categories: [names]`, `default_category` (default `Others`). Each
person has at most one category; a person without one is in the default category.

**Identity.** `identity` (`@user:domain`, from the feature-26 contact autocomplete) and `share_name`
(default on) are used by federation hints (§14).

**Settings:** `after_removal: reassign | hold` (default `reassign`), the categories, and scope.

### 7.2 Concepts

- **The concept tree** is the model's groups, with `parent_id`. A concept has optional `prompts`
  (text, any language), an `exclusive` flag (its children compete), a `threshold`, and a learned
  `probe`.
- **Examples are decisions.** Positives are pictures with a manual tag at or under the concept's path
  in the primary tree (a tag implies its ancestors). Negatives are its rejections.
- **Scoring** (`domain::grouping::concepts::score`, one Rust function used by both the pipeline and
  the routine):
  - **< 3 positives:** zero-shot, `sigmoid(t · (x·p) + b)`, max over prompts (`t`, `b` = SigLIP 2's
    logit scale and bias).
  - **≥ 3 positives:** a logistic-regression **probe** over the embedding, trained in the routine
    with the prompt embedding as an extra positive (closed-form-free gradient descent, a few hundred
    iterations, milliseconds).
  - A concept with no prompt and fewer than 3 positives scores nothing; only its manual tags count.
- **Hierarchy:** a child is scored only on pictures its parent holds.
- **Membership:** `in(G) = manual(G) ∨ (score ≥ threshold ∧ ¬rejected(G))`. In an exclusive parent,
  only the best child above threshold (a manual child always wins).
- **The routine** streams the in-scope embeddings once per run and scores every concept, so learning
  is retroactive. Only flipped pictures are invalidated.
- **Prompt embeddings** come from `ml_text_embeddings`. A miss is embedded by the backend text
  encoder (§13.1) and cached.
- **Starter list:** an instance setting (`ml_concepts_default_list`, multilingual: screenshot,
  document, receipt, whiteboard, meme…) offered when creating a concepts model.

### 7.3 Timeline

Input: owned **and received** in-scope pictures with `captured_at`; GPS when present. The routine
recomputes the whole partition (sorting is linear) and runs three steps in order:
1. **Zones** (home, work, parents' house): DBSCAN on geotagged pictures (`zone_radius_m`, default
   1000). A cluster is a zone when its pictures spread over ≥ `zone_min_days` distinct days across
   ≥ `zone_min_months` months, rather than a few multi-day blocks. Zones are named from the geocoder
   (`Lyon`, `Lyon_2`) and renamed by the user.
2. **Events:** time segmentation of the out-of-zone pictures (`max_gap_hours`, default 36), with
   `min_pictures` (default 10). An event may span places.
   - Pictures without GPS join an event by time adjacency.
   - A GPS-less stretch falls back to time-only bursts.
   - `events_in_zones` (off by default) also detects dense bursts inside zones.
3. **Series:** events whose main place (medoid of geotagged members) lies within `series_radius_m`
   (default 20000) of another event's form a series (≥ 2 events).

- **Names:** templates reusing segmentation placeholders plus `{place}`, `{country}`, `{region}`.
  Defaults: event `{place}_{year}_{month}`, series `{place}`, zone `{place}`. `{place}` is the
  dominant city, widening to region or country when members spread further. Slugs are ASCII.
- **Decisions:**
  - A picture manually tagged with an event or zone is fixed in it (newest tag wins; the model is
    exclusive).
  - A new label creates an event made of exactly its tagged pictures.
  - A rejection keeps a picture out of that group.
- **Stability:** new groups are matched to old ones by maximum overlap, so names, merges and splits
  survive. A filled gap that would merge two named events keeps both and flags it.
- **Place-level corrections** (board): *is a zone / is not a zone*, *split series*.
- **Settings:** the thresholds above, the name templates, `series_removal: picture | event | series`
  (default `picture`), and scope.

---

## 8. Grouping services (trees)

A service of type `grouping` projects one model into one tag tree. It computes nothing.

```jsonc
{ "model_id": "<people>", "primary": true,  "root": "People", "unmatched": "Unknown", "by_category": true }
{ "model_id": "<people>", "primary": false, "root": "Alone",  "faces_per_picture": { "min": 1, "max": 1 } }
{ "model_id": "<people>", "primary": false, "root": "Family", "select": { "categories": ["Family"] } }
```

| Option              | Meaning                                                                    | Default |
|:--------------------|:---------------------------------------------------------------------------|:--------|
| `model_id`          | the model shown                                                            | required |
| `primary`           | the editable tree (§2); at most one per model, validated on save           | on for a model's first tree |
| `root`              | where the tree goes; may not overlap any other grouping root               | required |
| `requires/excludes` | tree gates (the service's existing columns)                                | none |
| `select`            | `include` / `exclude` group ids, plus kind filters below                    | all |
| `layout`            | nesting levels above the group                                             | flat |
| `unnamed`           | also tag auto-named groups (`Person_N`, …)                                  | on |
| `unmatched`         | slug of a bucket for in-scope pictures with **no tag under the root from any source** (people: and an eligible free face) | people primary: `Unknown`; else none |
| `min_group_size`    | groups with fewer members produce no tag                                   | 1 |

| Kind     | Kind options                                                                                     |
|:---------|:-------------------------------------------------------------------------------------------------|
| people   | `by_category` (nest under the category; no category → the default category: `People.Family.Alice`, `People.Others.Carol`); `select.categories`; `faces_per_picture {min, max}` over eligible unflagged faces |
| concepts | `select.subtree` (a concept); the concept tree is the layout                                     |
| timeline | `select.kinds` (events / zones); `select.min_days`; `select.abroad_only` (country ≠ the main zone's); `layout` ⊂ {`country`, `region`, `year`, `series`} in any order; `leaf: event \| series` |

- **Hidden groups** (a model-level flag set from the board) produce no tag in any tree, unless a
  tree names them in `select.include`.
- **Timeline layout:** the `series` level is skipped for an event outside any series;
  `leaf: series` tags a whole series as one; a multi-country event sits under its dominant country.
- **Derived levels** (`year`, `country`, `region`, `category`) are renamed only through
  `tag_metadata.display_name`. Renaming the level tag is refused (409).
- **A filtered primary stays editable.** A manual tag naming a group the primary filters out still
  asserts it; that tree just shows it as manual-only.
- **Read-only trees:** their grouping tags are not removable (no ×, WebDAV 409). Manual tags under
  their roots are allowed and ignored by the model.
- **No primary:** the model still learns and its page works. Face edits write pins only, and no
  manual tag is a decision.

### 8.1 Common cases

**People** (one model; each row is a tree unless it says *rule* or *hierarchy*)

| Case                                    | Representation                                                    | Tags |
|:----------------------------------------|:------------------------------------------------------------------|:-----|
| everyone by name                        | primary, flat                                                     | `People.Alice` |
| not identified yet                      | `unnamed` on; `unmatched: Unknown`                                | `People.Person_12`, `People.Unknown` |
| strangers / people to leave out         | board *hide*                                                      | — |
| me                                      | a person linked to my own `@identity`                             | `People.Me` |
| nested by category                      | categories + `by_category`                                        | `People.Family.Alice`, `People.Others.Carol` |
| one category as its own sharable tree   | tree `{root: Family, select.categories: [Family]}`                | `Family.Alice` |
| a hand-picked set                       | tree with `select.include`                                        | `Close.Alice` |
| someone alone                           | tree `{root: Alone, faces_per_picture: {1, 1}}`                   | `Alone.Alice` |
| group shots per person                  | tree `{root: Group_shots, faces_per_picture: {min: 3}}`           | `Group_shots.Alice` |
| no crowds in the main tree              | primary `faces_per_picture: {max: 10}`                            | — |
| a person by year                        | a segmentation service rooted at `People.Alice` (subtags are free) | `People.Alice.2019` |
| portraits / group shots, flat           | *rule* `face_count = 1` / `≥ 5`                                   | `Portraits` |
| Alice with Bob                          | *hierarchy* query node, or a *rule*                               | — |
| family gathering                        | *rule* `people_count(category: Family) ≥ 4`                       | `Family_gatherings` |

**Concepts**

| Case                                    | Representation                                                    | Tags |
|:----------------------------------------|:------------------------------------------------------------------|:-----|
| kinds of pictures                       | concepts model + primary tree                                     | `Kinds.Screenshot` |
| sub-kinds                               | nested concepts                                                   | `Kinds.Document.Receipt` |
| one kind per picture                    | `exclusive` on the parent                                         | — |
| one subtree as its own tree             | tree with `select.subtree`                                        | `Documents.Receipt` |
| an independent grouping                 | another concepts model                                            | `Subjects.Cars` |
| everything else                         | `unmatched: Other`                                                | `Kinds.Other` |
| a static detector, no learning          | *rule* `concept_score("sunset") ≥ 0.8`                            | any |

**Timeline**

| Case                                    | Representation                                                    | Tags |
|:----------------------------------------|:------------------------------------------------------------------|:-----|
| trips by year / by country / both       | `layout: [year]` / `[country]` / two trees on one model           | `Trips.2024.Rome_2024_05`, `Countries.Italy.…` |
| recurring places                        | `layout: [series]`; `leaf: series` for one tag per place          | `Trips.Brittany_House.Brittany_2023_08` |
| home, work, parents'                    | tree `{select.kinds: [zones]}`                                    | `Zones.Home` |
| only real trips / abroad only           | `select.min_days`, `min_group_size` / `select.abroad_only`        | — |
| birthdays at home                       | model `events_in_zones` + a tree                                  | `Events.2024_06_12` |
| an independent timeline                 | another timeline model                                            | `Their_trips.…` |

---

## 9. Projection in the pipeline

`routines::pipeline::evaluation::run_for_user` gains:

1. **Per batch:**
   - the batch's analyses (by `ml_key`), faces and flags;
   - per model in use (referenced by an enabled grouping tree, or by a rule with people or concept
     predicates): model state, rejections, pins and derived members for the batch.
2. **Per picture, per model, lazily on first need:** `solve` returns the picture's members.
3. **At a grouping tree's position:**
   - check scope gates ∧ tree gates on the accumulated gating tags;
   - emit `root + layout + slug` for each member passing `select` / hidden / `min_group_size` /
     `faces_per_picture`;
   - then the `unmatched` bucket if due;
   - feed the outputs to `GateHistory` like any service.
4. **Rule predicates** read the same per-picture solution (§11).
5. **Handshake:** if a solution differs from the batch's derived members, mark the model for a
   debounced routine wake after the batch.

### 9.1 Decisions as read by the solver

The **asserted** groups of *p* are the groups named by *p*'s manual tags under the primary root:
- skip the tree's layout levels;
- take the group label;
- a deeper manual tag asserts its group, and its suffix is ignored by the model.

Manual tags under read-only roots are ignored. A manual tag beats a rejection of the same group.

### 9.2 The people solver

Inputs: *p*'s faces (embedding, eligible, bbox), flags, the model's pins on them, the asserted
people, the rejected people, prototypes, thresholds.

```
1. flagged faces are out; pinned faces are fixed (person, or "hold" = never auto-assigned)
2. asserted people not already pinned are placed on free faces (a face is free unless pinned or
   flagged; automatic picks do not reserve it):
     joint assignment (kuhn_munkres), maximising total similarity, every asserted person mandatory
     a person WITH prototypes is placed when its face is ≥ SIM_PLAUSIBLE
     a person WITHOUT prototypes is placed only when FORCED:
       - as many free faces remain as unplaced asserted people, with at most one of them new
       - or exactly one free face is ORPHANED (its nearest person is rejected on p)
     anyone left is "face unknown" (asserted, no face)
3. remaining free ELIGIBLE faces get automatic people: nearest prototype ≥ SIM_AUTO, excluding
   people already on p and people rejected on p, each person at most once. An orphaned face
   follows after_removal: reassign (as above) or hold
4. faces left unplaced are the pool
```

- **Capacity** falls out of step 2: on a one-face picture, a manually tagged Bob takes the face from
  an automatic Alice.
- Two manual people on a one-face picture both keep their manual tags; only the better match is
  placed.
- **Members:** placed people (`how` = pin / manual / auto). Face-unknown people are members with
  `face_id = NULL, how = manual`. They are emitted as tags too, so the tree shows M+S; the board lists
  them as *face unknown*.
- **`unmatched`** is due when *p* has an eligible free face and no row of any source under the root.

### 9.3 The concepts and timeline solvers

- **Concepts:** §7.2 membership, per picture, from the embedding and the model state.
- **Timeline:** the asserted group if any; else the cached partition member unless rejected; else
  none.

---

## 10. Corrections

### 10.1 Tag states

For a picture *p* and a path P under a primary root R:

| State   | Rows                                    | Meaning |
|:--------|:----------------------------------------|:--------|
| **S**   | `grouping`                              | the model placed it |
| **M**   | `manual`                                | the user says so; the model has not placed it (people: face unknown) |
| **M+S** | both                                    | agreed; reads fold them into one path, and provenance shows both |
| **+L**  | plus a locked row (rule, segment, share) | P stays whatever happens to S / M |

Stale paths are accepted: after a layout change or a re-dated event, a manual tag keeps its old path
(`Trips.2023.Rome`) while S moves on. It still asserts its group by label. Tidying is a later,
explicit user action (§20).

### 10.2 Adding

**Nothing intercepts a manual add.** Every add path (`TagRepository::assign_inner`: tag editor, batch,
WebDAV transitions, uploads, dedup handoffs) already invalidates the picture. The next pipeline run
solves it with the new decision.

### 10.3 Removing

`services::grouping::reject_under(tx, user, picture_ids, paths)` is called in the existing
transaction, before `TagRepository::batch_remove`, from the **two** user-intent removal paths:
`services::tags::batch_edit_tags` and `services::vfs::dirops::apply_transition`. For each path T
under a **primary** root, on each picture, it:
1. records a **rejection** for every group whose projected path, or asserting manual tag, lies at or
   under T;
2. drops that model's pins to those people on the picture;
3. deletes the matching `grouping` rows at once and invalidates the picture, so a WebDAV client never
   sees the file linger after its MOVE;
4. **refuses** the `unmatched` bucket and paths under **read-only** roots:
   - batch: counted as not removable;
   - WebDAV DELETE: 409;
   - exception: inside a WebDAV transition that also adds under the same root
     (`Unknown/x` → `Bob/`), the bucket's removal is a no-op.

After removing T, no grouping tree produces anything at or under T for *p*, except the `unmatched`
bucket, which may reappear. A later manual tag for the group wins over the rejection; the routine
garbage-collects rejections shadowed this way.

**Hardcoding inventory:**

| Place | Change |
|:------|:-------|
| `batch_edit_tags`, `apply_transition` | call `reject_under` before `batch_remove` |
| `apply_transition` | the 409 check becomes `has_locked_tag_under`: non-manual rows other than primary grouping rows, plus `unmatched` unless the transition adds under its root |
| `count_selection_with_manual_under`, `aggregate_tags` | `removable_count` = manual ∪ primary grouping, minus `unmatched` |
| tag sources (`GET /tags?with_sources`, picture detail) | each source carries `removable: bool` |
| `services::tags::cascade_rename` + `POST /tags/rename` handler | the handler validates through `grouping::validate_rename` (409 on derived levels, read-only roots, or leaving the root); the cascade calls `grouping::rename` |
| frontend `SelectionPanel`, `MultiSelectionPanel` | read `removable` / `removable_count` instead of testing `'manual'` |

### 10.4 Rename, merge, delete

- **Rename a group path** under the primary (tag rename, or board rename):
  - target label free → slug rename; the cascade renames the manual rows;
  - target label is an existing group of the same parent → **merge**: rejections, pins and
    derived members move to the target, `merged_into` is set, prototypes are rebuilt;
  - renaming the root → the existing config rename rewrites `root`.
- **Board delete** (after confirmation): deletes the group's manual tags under the primary (else they
  would re-seed it), its rejections and pins. Its faces return to the pool. To silence a group, hide
  it instead.

### 10.5 The face API

`GET /pictures/{id}/faces` returns:
- boxes, eligibility, flags;
- for the people model: each face's person and how (pin / manual / auto, confidence);
- the face-unknown people.

`PUT /pictures/{id}/faces/{face_id}` takes `{ "person": <id> | {"new": "<label>"} | "not_a_person" |
null }` (`null` = unpin and unflag). One transaction, through the same tag services, so tags stay the
record everywhere. It returns the re-solved faces.

| Action | Pin / flag | Tags (under the primary root, when one exists) |
|:-------|:-----------|:-----------------------------------------------|
| f → Bob | pin f → Bob | add manual `Bob`. The previous person keeps their manual tag and is re-placed, or becomes face unknown. **On a one-face picture only**, the previous person's manual tag is removed (+ rejection) |
| f → new "Dan" | create Dan, pin | add manual `Dan`; previous person as above |
| f → *not a person* | flag (per user) | none; f stops counting as a face, and a manual previous person becomes face unknown |
| unpin | drop pin and flag | none |
| *use as cover* | — | the person's `tag_metadata` cover + `cover_focus` (§12) |

### 10.6 Behaviour catalogue (acceptance cases)

`People` primary (`Unknown` on), a WebDAV mirror over `People`, and a *Family* hierarchy node (a
`static` node with one `mirror` per person). *f1*, *f2* are *p*'s faces.

| #   | Start | Action | End |
|:----|:------|:-------|:----|
| P1  | f1 auto Alice, f2 pool | add `People.Bob` (known, plausible on f2) | Alice(S), Bob(M+S) |
| P2  | one face, f1 auto Alice | add `People.Bob` (known or new) | Bob(M+S); Alice gone (forced) |
| P3  | one face, f1 pinned Alice | add `People.Bob` | Alice(S), Bob(M) face unknown |
| P4  | one face, Alice(M+S) | add `People.Bob` | both M; S on the better match |
| P5  | f1 auto Alice 0.6, f2 auto Carol 0.9 | add `People.Bob` (known) | Carol(S), Bob(M+S); Alice gone |
| P6  | same | add `People.Bob` (new) | Alice(S), Carol(S), Bob(M) |
| P7  | Unknown(S), f1 pool | add `People.Bob` | Bob(M+S) |
| P8  | Unknown(S), f1, f2 pool | add `People.Bob` (new) | Bob(M); Unknown gone |
| P9  | Alice(S) | add `People.Alice` | Alice(M+S); f1 anchored |
| P10 | Alice(S) | add `People.Alice.Childhood` | Alice(S) + Alice.Childhood(M) |
| P11 | f1 auto Alice | remove `People.Alice` | rejection; f1 orphaned → reassign (Person X) or hold → Unknown |
| P12 | Alice(M+S) | remove `People.Alice` | manual deleted + rejection; as P11 |
| P13 | Unknown(S) | remove `People.Unknown` | refused |
| P14 | Alice(S) + a rule asserting `People.Alice` | remove | rejection; Alice(L) stays |
| P15 | read-only `Alone.Alice` | remove | refused |
| W1  | — | MOVE `People/Alice/x` → `People/Bob/` | f1 orphaned → Bob(M+S), even when Bob is new |
| W2  | — | MOVE `People/Unknown/x` → `People/Bob/` | as P7 / P8 |
| W3  | one-face picture | COPY `People/Alice/x` → `People/Bob/` | acts as a move (capacity) |
| W4  | — | DELETE `People/Unknown/x` | 409 |
| W5  | — | PUT into `People/Bob/` | Bob(M) until analysed, then solved |
| W6  | — | MOVE `Family/Alice/x` → `Family/Bob/` | the folders are `People.*` tags: as W1 |
| F1  | f1 Alice(M+S), f2 auto Carol | f1 → Bob | Bob(M+S); Alice re-placed on f2 if she fits better than Carol, else Alice(M) |
| F2  | one face, Alice(M+S) | f1 → Bob | Alice's tag removed + rejection; Bob(M+S) |
| F3  | f1 manual Alice | f1 → *not a person* | Alice(M) face unknown; f1 out of capacity and `Unknown` |
| G1  | — | rename `People.Person_12` → `People.Alice` (free) | slug rename |
| G2  | — | same, Alice exists | merge |
| G3  | — | rename `Trips.2024` | 409 |
| D1  | — | Bob tagged on 3 group photos, face unknown | bootstrap gives Bob prototypes; the pictures become M+S |
| D2  | — | `Person_7` within `SIM_MERGE` of named Alice | auto-merged |
| C1  | — | add `Kinds.Screenshot` | a positive; Screenshot(M+S); in an exclusive parent, manual wins |
| C2  | — | add `Kinds.Van` (new) | concept Van seeded |
| C3  | — | remove `Kinds.Screenshot` | rejection = a negative; only flipped pictures re-run |
| T1  | *p* auto in Rome | add `Trips.2024.Naples_2024_05` | fixed in Naples; Rome gone |
| T2  | — | add `Trips.Day_at_lake` (new) | a new event of exactly its tagged pictures |
| T3  | — | remove a series-level tag | per `series_removal` |

---

## 11. Rule predicates (feature 13)

New `domain::predicate` leaves, evaluated on `PipelineInput`, which gains an `ml` block filled from
the batch:

| Leaf | Kind | Reads | Phase |
|:-----|:-----|:------|:------|
| `face_count`, `unknown_face_count` | `Field` (Int) | eligible unflagged faces; pool faces | 6 |
| `person(id)` | `Predicate::Person` | the people model's members for *p* | 6 |
| `people_count(ids \| category) {min, max}` | `Predicate::PeopleCount` | same | 6 |
| `concept_score(prompt) ≥ t` | `Predicate::ConceptScore` | embedding vs the prompt embedding (cache) | 4 |
| `similar_to(picture_id) ≥ t` | `Predicate::SimilarTo` | embedding vs the reference picture's | 9 |
| `place_country`, `place_region`, `place_city` | `Field` (Str) | offline geocoding of GPS | 11 |

- **No leaves name a concept or timeline group.** Concepts are queried by text, and those results
  are tags that rules reach through gates.
- People leaves need no service reference (one people model per user).
- They are order-independent: they read the per-picture solution, not tags.
- A picture outside the people scope, or with ML off, reads as missing.

---

## 12. Covers

There is **no separate face-thumbnail system**. A group's picture is its tag's feature-34 cover.
- **`tag_metadata.cover_focus`** is an optional normalised box in the raw-pixel frame. It travels with
  the cover in share metadata.
- **The cover component** (`OrientedCoverImage`) crops to the focus, inside the existing orientation
  transform. It picks the smallest variant where the box is ≥ 96 px.
- **Set as thumbnail** (selection panel, or faces mode *use as cover*) fills `cover_focus`
  automatically when the tag is a person placed on a face of that picture.
- **Default cover** when none is set (revisits feature 34 §2's "no auto-derived cover"):
  1. the explicit `tag_metadata` cover;
  2. for a group path: the group's derived cover, computed by its routine:
     - person: most typical face, the eligible face nearest the main prototype, with its box;
     - concept: the highest score;
     - event: the temporal middle;
     - zone: the spatial centre;
     - series: its latest event's cover;
  3. for any other tag: the most recent capture under it, from one `DISTINCT ON (tag_path)` over the
     user's direct tags, cached with the tag payload. The client walks down to the first child's
     cover for an ancestor with no direct picture.
- Derived covers are local (never shared), may be received pictures, and are veiled by the
  feature-38 curtain like any cover.

---

## 13. Semantic search

### 13.1 Backend text encoder

`services::ml::text` (in `AppState`) holds the SigLIP 2 text tower:
- loaded lazily on first use, and unloaded after `ml_text_idle_unload_secs` (300) without use;
- behind `Arc<dyn TextEmbedder>`, so tests inject `archypix_ml::stub`;
- every embedding cached in `ml_text_embeddings`.

### 13.2 Search

`GET /pictures` gains:
- `semantic=<text>` with sort `semantic_near`;
- `similar_to=<picture_id>` with the same sort.

Both compose with `push_filters` like feature 29's proximity sorts, so filters, selections and batch
actions all work. Rows without an analysis are excluded under these sorts. Per user, the query is an
**exact** scan over the user's analysed rows (fine to ~100k), not a global HNSW followed by a
filter. A `score` field is returned per item.

---

## 14. Federation of analyses & identity hints

- **New verb** `PicturesMlAnalyses` on the feature-28 envelope, with its own `VERSION`.
  - **Request:** `{ items: [{ picture_token, ml_key }] }`.
  - **Response per item:** `analysis {model_set, embedding, faces[], hints[]}` | `not_yet` |
    `unavailable` (owner ML off, or a model-set mismatch).
- **Owner side:** resolve the token to the owner's picture, check the key matches, and serve the
  analysis. Faces carry **hints** `{face_index, bbox, identity | name}` for faces placed on a person
  who is:
  - the owner (identity = own handle);
  - linked to an `@identity`;
  - free-text named with `share_name` on.

  Hints are only sent when the owner's faces are enabled.
- **Recipient side:** step 0 of `ml_drain` for received pending rows asks each origin owner in
  batches of 100. A transitive recipient asks the origin, never the relayer.
  - `analysis` → stored with `origin = remote` after validation (dimensions, finite, unit norm ± ε),
    then recorded as in §5.2.
  - `not_yet` → back off (`ml_retry_at`); after 24 h, analyse locally.
  - `unavailable` or owner offline → analyse locally (owner offline: retry first).
- **Applying hints** (recipient people model, in-scope pictures):
  - a hinted face joins the person carrying the same identity at `SIM_PLAUSIBLE` (soft must-link);
  - an unnamed person whose faces carry one consistent hint is named from it (the identity's
    username, or the free-text name);
  - user-chosen names are never overridden;
  - conflicting hints → board suggestions only.
- **Trust:** remote vectors and hints are untrusted input. At worst they cause a wrong tag or a wrong
  suggestion on a received picture.

---

## 15. API summary

All under `/api/authenticated` unless noted; documented in 06 by the phase that adds them.

| Endpoint | Phase |
|:---------|:------|
| `GET /ml/status` — the user's `ml_status` counts (progress indicator) | 2 |
| `GET /admin/ml` — backlog, oldest pending age, analyses / faces counts, storage | 2 |
| `GET/POST /grouping-models`, `GET/PATCH/DELETE /grouping-models/{id}` (name, scope, settings) | 3 |
| `GET /grouping-models/people` — get-or-create the people model | 6 |
| `GET /grouping-models/{id}/groups` — slug, display name, parent, hidden, count, cover, kind data | 3 |
| `POST /grouping-models/{id}/groups` — create a concept (prompts, parent) | 3 |
| `PATCH /grouping-models/{id}/groups/{gid}` — rename, hidden, prompts / threshold / exclusive, category / identity / share_name | 3 / 6 |
| `POST /grouping-models/{id}/groups/{gid}/merge` `{into}` | 3 |
| `DELETE /grouping-models/{id}/groups/{gid}` | 3 |
| `GET /grouping-models/{id}/pictures?group=&state=` — `PictureListItem` pages (`state`: members / face_unknown / pool / unmatched) | 3 / 6 |
| `GET /pictures/{id}/faces`, `PUT /pictures/{id}/faces/{face_id}` | 6 |
| `GET /pictures?semantic=…|similar_to=…&sort=semantic_near` | 9 |
| tagging-service create/update accepts `service_type: grouping` (+ validation) | 3 |
| `POST /tags/rename` — grouping validation (409s) | 3 |
| tag sources `removable`, aggregate `removable_count`, tag payload `cover` resolution + `cover_focus` | 3 / 4 |
| federation `PicturesMlAnalyses` | 8 |

---

## 16. Frontend

Reuse first (05 §11, AGENTS.md):
- **Grids** are `PhotoGrid` with a `pictureSource` built on `GET /grouping-models/{id}/pictures`.
- **Viewing** is the existing `Lightbox`.
- **Covers** are `OrientedCoverImage`.
- **Service editing** extends `components/tagging` (`serviceTypes.ts`, `NewServiceMenu`,
  `ServiceEditor`).

| Surface | Phase |
|:--------|:------|
| Removable flags in `SelectionPanel` / `MultiSelectionPanel`; `grouping` source label and colour | 5 |
| `grouping` service editor: model picker / "new model", primary toggle, root, select, layout, unnamed, unmatched, min size, kind options | 5 (generic + concepts), 7 (people), 12 (timeline) |
| `/models` (list), `/models/:id` **model page**: settings + scope gates, the **group board** (groups with covers and counts, hide / rename / merge / delete, concept tree with prompts / threshold / exclusive), members grid | 5 |
| Focused and default covers in `OrientedCoverImage` / `SubtagBlock` | 5 |
| ML progress indicator (from `/ml/status`); admin ML card; user *faces* toggle | 5 |
| `/people` (the people model page on the same component): categories, identity / share_name, face-unknown and pool grids, suggestions | 7 |
| Lightbox **faces mode** (`F`, top-bar toggle): `FaceOverlay` boxes inside `ZoomableArea`, popover (person picker, new person, not a person, unpin, use as cover), face-unknown chips | 7 |
| Details panel **Faces** section (people with face crops, face-unknown chips, unknown count, *Edit faces*) | 7 |
| `/search` page (text box, results grid, "more like this" action in the Lightbox and the selection panel) | 10 |
| Timeline model page specifics (zones / events / series board, place corrections, GeoNames attribution) | 12 |

---

## 17. Settings, privacy, progress

**Instance settings** (DB-editable, group *ML*):

| Key | Default |
|:----|:--------|
| `ml_enabled` | true |
| `ml_faces_enabled` | true (detection itself; off means no faces are computed instance-wide) |
| `ml_faces_default_for_new_users` | true |
| `ml_drain_interval_secs`, `ml_drain_batch` | 60, 500 |
| `ml_gc_interval_secs` | 86400 |
| `ml_text_idle_unload_secs` | 300 |
| `ml_models_dir` (env-only, core) | `/models` |
| `ml_concepts_default_list` (JSON) | the starter list |

- **Worker settings:** `ML_ENABLED` (true), `ML_MODELS_DIR` (`/models`), `MAX_CONCURRENT_ML_JOBS`
  (1).
- **Compose:** a `ml_models` volume mounted at `/models` in the backend and worker services.
- **User setting:** `faces_enabled` (default from the instance). Turning it off **deletes** the people
  model (groups, pins, rejections, derived members) and the user's face flags. Manual `People.*` tags
  stay: they are ordinary tags.
- **Progress:** `/ml/status` counts drive an "N pictures awaiting analysis" indicator.
- **GDPR (not legal advice):**
  - Face embeddings used for identification are Art. 9 biometric data; individuals organising their
    own photos fall under the household exemption.
  - A hosted provider keeps faces opt-out-able and deletable, with a default-off option for stricter
    jurisdictions.
  - Analyses and hints only travel to recipients holding the picture token, the same audience as the
    pixels.
  - Free-text names travel only with the per-person `share_name` switch.

---

## 18. Edge cases

- **Trash:** analyses are kept (restore is free); trashed rows are out of models, search and drains;
  a purge leads to GC.
- **Dedup / copies:** the analysis is shared by `ml_key`. Decisions are per picture row, like tags
  (the dedup handoff moves the manual tags).
- **Non-thumbnailable formats** → `unsupported`. Videos use the poster frame (often faceless).
- **Faces in posters, screens, statues; children ageing; twins; masks:** eligibility, *not a
  person*, pins and prototypes per period. No auto-merge between named people.
- **Sensitive pictures (38):** covers, face crops, boards and search results honour the curtain.
- **Public shares (27):** never expose people, concepts, timeline data or vectors. Anonymous
  contributions are analysed like owned pictures.
- **Timeline:** undated pictures join nothing. A wrong camera clock shifts gaps, and fixing the date
  (feature 30) re-runs the model. A time-zone-naive `captured_at` can split or merge long-haul trips.
- **Multi-replica backends:** model routines need the same per-key exclusivity as the pipeline (the
  separate multi-replica follow-up).

---

## 19. Phases

Phases run in numeric order. **Depends** lists the hard prerequisites (useful if the order is ever
changed). Each phase lists **Depends**, **Scope**, **Tests** (what must pass) and **Docs**. "Done"
means: the
workspace builds (`cargo check --workspace --tests`, `SQLX_OFFLINE=true` for back), every existing
and new test passes, and for front phases `npm run build` passes with lint clean on changed lines.

### Phase 1 — `archypix-ml` crate and worker `ml_analyze`
- **Depends:** —
- **Scope:**
  - New crate `archypix-ml` (§4.4): registry with the four model files pinned, preprocessing
    (orientation, resize, normalise, 5-point alignment), thresholds, traits, `onnx` engines behind the
    feature, `stub` engines, `analyse`.
  - `common`: `JobType::MlAnalyze`, `JobConfig::MlAnalyze`, `JobProduct::Ml(MlAnalysis)`; delete the
    three stubs.
  - Migration: rebuild the `job_type` enum (delete stub rows first), `jobs.priority`, and
    `pictures.ml_key` (generated, §5.1) + `ml_status` + index. No pgvector yet.
  - `JobRepository::claim_next` orders by `priority, created_at`.
  - Claim for `ml_analyze` binds `ml_key` / orientation and presigns the large thumbnail (owned, and
    received through `presign_variant_for_picture`).
  - `respond_job` accepts the ML product (stored in the job result only).
  - Worker: `jobs/ml.rs`, ML settings, engine loading with capability on `/health`,
    `MAX_CONCURRENT_ML_JOBS` via `ClaimQuery.types`.
  - Worker Dockerfile ships the ONNX Runtime.
  - Compose `ml_models` volume.
- **Tests:**
  - unit: preprocessing (orientation round-trip of a box, alignment shape), `analyse` with stubs,
    eligibility;
  - `back/tests/worker_contract.rs`: an inserted `ml_analyze` job claims with a large-thumbnail URL
    and accepts an `Ml` product;
  - claim priority order;
  - `#[ignore]` real-model test: a sample face image gives ≥ 1 eligible face and unit-norm vectors.
- **Docs:** 04 (module layout, ML job), 02 (worker ML capability), 06 §8 (worker endpoints).

### Phase 2 — analyses in the backend
- **Depends:** 1
- **Scope:**
  - Migration: `CREATE EXTENSION vector`, `ml_analyses`, `ml_faces` (+ HNSW), `ml_text_embeddings`.
  - `pgvector` in back.
  - `services::ml::record_analysis` + invalidation of all rows sharing the key.
  - `ml_drain` routine (reuse / unsupported / stale / enqueue, round-robin) and its triggers (thumbnail
    completion, ingest).
  - `ml_gc` sweep.
  - Instance settings (§17).
  - `GET /ml/status`, `GET /admin/ml`.
  - A `ModelRoutines` wake hook in `Routines` with no models yet (a no-op registry phase 3 fills).
- **Tests** (DB, `back/tests/ml_analyses.rs`, stub vectors):
  - recording sets every sharing row `analyzed` and dirty;
  - a second job for the same key reuses;
  - an orientation change makes the row stale, then pending;
  - unsupported rows;
  - GC;
  - drain round-robin;
  - `ml_enabled = false` → no jobs.
- **Docs:** 03 §E/§H (drain, GC, invalidation), 06 (`/ml/status`, `/admin/ml`), 01 (analysis model, a
  short paragraph).

### Phase 3 — models framework, grouping trees, concepts (zero-shot)
- **Depends:** 2
- **Scope:**
  - Migration: `grouping_models`, `model_groups`, `rejections`, `derived_members`, enum values
    (`service_type` grouping, `tag_source` grouping).
  - `ServiceType::Grouping`, `ServiceConfig::Grouping(GroupingConfig)` parse + validate (model exists
    and belongs to the user, single primary, root overlap, kind options).
  - Pipeline integration (§9): batch loading, lazy per-model solve, projection, `unmatched`,
    `GateHistory` feed, handshake.
  - The generic model routine (per model key) + routines registry.
  - Concepts model: tree, prompts, zero-shot scoring, seeding, exclusive, members.
  - Backend text encoder service (§13.1) + prompt cache; the back Dockerfile ships the ONNX Runtime.
  - Corrections: `reject_under` in both removal paths, `has_locked_tag_under`, removable flags /
    `removable_count`, `validate_rename` + `grouping::rename` / merge, board delete.
  - Model / group / pictures endpoints (§15).
- **Tests** (DB, `back/tests/grouping.rs`, stub text and vectors):
  - primary uniqueness and root-overlap validation;
  - C1–C3;
  - P13 / P15-style refusals on a concepts tree (unmatched, read-only);
  - WebDAV removal → rejection and 409s (`vfs.rs` suite);
  - rename / merge / derived-level 409;
  - handshake convergence: a manual tag → pipeline → routine → stable after one more run;
  - scope gates vs tree gates;
  - subtags from a segmentation under a group are ignored.
- **Docs:** 01 §3 (grouping service, models, removable grouping tags), 03 §E (projection,
  handshake), 06 (models, groups, service config, rename 409s, removable fields). 05 comes with
  phase 5.

### Phase 4 — concept learning, `concept_score`, covers (backend)
- **Depends:** 3
- **Scope:**
  - Probe training (≥ 3 positives) and retroactive scoring by streaming.
  - Concept threshold calibration note.
  - `Predicate::ConceptScore` (+ editor-facing validation).
  - Covers: `tag_metadata.cover_focus`, derived group covers in the routines, tag payload cover
    resolution (explicit → group → most recent), set-as-thumbnail focus fill.
- **Tests:**
  - a probe flips a borderline picture after 3 positives, and only flipped pictures are invalidated;
  - `concept_score` matches above / below threshold;
  - cover resolution order;
  - `cover_focus` round-trip and share propagation.
- **Docs:** feature 34 (§2 revisited, `cover_focus`), feature 13 (new leaf), 06 (tag payload cover),
  01 (concepts paragraph).

### Phase 5 — frontend: grouping trees, model pages, concepts, covers, ML status
- **Depends:** 4
- **Scope:** §16 rows marked 5 — removable flags, the grouping service editor (generic + concepts),
  `/models` + model page with the group board and concept tree editing, members grid via
  `pictureSource`, covers with focus and defaults, `concept_score` in `PredicateBuilder`, progress
  indicator, admin ML card, user faces toggle.
- **Tests:** `npm run build`; lint clean on changed lines. Manual check list for the user: create a
  concepts model + primary tree, tag / untag a picture, see the board update.
- **Docs:** 05 (routes, components, the board), 06 cross-check.

### Phase 6 — people model (backend)
- **Depends:** 3, 4 (runs after phase 5 in sequence)
- **Scope:**
  - Migration: `face_pins`, `face_flags`; get-or-create people model.
  - Solver (§9.2) with `kuhn_munkres`; prototypes; incremental routine (KNN on changed prototypes);
    discovery (HDBSCAN); bootstrap; auto-merge.
  - Categories / default category; identity / share_name fields.
  - `after_removal`.
  - People tree options (`by_category`, `faces_per_picture`, `select.categories`).
  - Face API (§10.5).
  - Pin remap on re-analysis.
  - Derived person cover with focus.
  - Predicates `face_count`, `unknown_face_count`, `Person`, `PeopleCount`.
  - `faces_enabled` user setting + deletion.
  - `SIM_*` calibration (`#[ignore]` test) recorded in §19.
- **Tests** (DB, `back/tests/people.rs`, stub face vectors constructed to be near or far):
  - P1–P12, P14, W1–W6, F1–F3, G1–G2, D1–D2 from §10.6;
  - the bootstrap pure function;
  - solver unit tests (forced, orphaned, capacity);
  - `faces_enabled = false` deletes the model.
- **Docs:** 01 §3 (people), 03 (people routine), 06 (face API, people endpoints), feature 13 (leaves).

### Phase 7 — people frontend
- **Depends:** 6
- **Scope:** §16 rows marked 7 — `/people` page (categories, identity / share_name, face-unknown /
  pool grids, suggestions), Lightbox faces mode + `FaceOverlay`, details Faces section, people
  options in the service editor, people predicates in `PredicateBuilder`.
- **Tests:** `npm run build`; lint; a manual check list (relabel a face, *not a person*, unpin, use
  as cover, `F` toggling across ←/→).
- **Docs:** 05 (Lightbox faces mode, Faces section, People page).

### Phase 8 — federation of analyses and identity hints (backend)
- **Depends:** 6
- **Scope:** the `PicturesMlAnalyses` verb (owner serve + hints; recipient pull as drain step 0,
  `ml_retry_at`, 24 h fallback, validation), applying hints in the people routine.
- **Tests** (federation suite, two backends): received rows get the owner's analysis without a job;
  `not_yet` back-off then local fallback; `unavailable` → local; hints name an unnamed recipient
  person but never override a chosen name; invalid vectors are rejected.
- **Docs:** 01 §6 (analyses + hints travel), feature 28 (new verb), 06 §8.

### Phase 9 — semantic search (backend)
- **Depends:** 3 (text encoder, prompt cache)
- **Scope:** `semantic` / `similar_to` list params, `PictureSortField::SemanticNear`, per-item
  `score`, `Predicate::SimilarTo`.
- **Tests:** ranking with stub vectors (nearest first), composition with a tag filter and a
  selection, rows without analyses excluded, `similar_to` leaf.
- **Docs:** 06 (list params), feature 29 (new proximity sort), feature 13 (leaf).

### Phase 10 — semantic search (frontend)
- **Depends:** 9
- **Scope:** the `/search` page (text box → grid with scores, filters reuse), a "more like this"
  action in the Lightbox and the selection panel, `similar_to` in `PredicateBuilder`.
- **Tests:** `npm run build`; lint; a manual check list.
- **Docs:** 05.

### Phase 11 — timeline (backend)
- **Depends:** 3, 4 (covers)
- **Scope:**
  - GeoNames loader (download to `ml_models_dir`, sha256, k-d tree, ASCII names).
  - `place_*` predicate fields.
  - Timeline model (zones, events, series, names, stability matching, decisions, place-level
    corrections, `series_removal`).
  - Timeline tree options and layouts.
  - Derived covers.
- **Tests:** synthetic timelines (home cluster + two trips + a recurring place): zones / events /
  series detected; layouts; T1–T3; stability under a new picture filling a gap; `abroad_only`;
  `place_*` leaves.
- **Docs:** 01 §3 (timeline), 06, feature 20 (template reuse), feature 13 (place fields).

### Phase 12 — timeline frontend
- **Depends:** 11
- **Scope:** the timeline model page (zones / events / series board, place corrections, GeoNames
  attribution), timeline options in the service editor, `place_*` fields in `PredicateBuilder`.
- **Tests:** `npm run build`; lint; a manual check list.
- **Docs:** 05; roadmap: ML workers → done.

### Checklist

- [ ] Phase 1  - [ ] Phase 2  - [ ] Phase 3  - [ ] Phase 4  - [ ] Phase 5  - [ ] Phase 6
- [ ] Phase 7  - [ ] Phase 8  - [ ] Phase 9  - [ ] Phase 10 - [ ] Phase 11 - [ ] Phase 12

*Deviations and calibration results are recorded here by each phase.*

---

## 20. Later, postponed, rejected

**Later:**
- **Model-set upgrade.** Analyse under the new set while serving the old, remap pins by IoU,
  re-embed prompts, retrain probes, drop the old set. Federation peers on another set answer
  `unavailable`.
- **User-drawn faces:** a per-user `user_faces` row plus an `ml_embed_faces` job that embeds given
  boxes. They count for capacity and can be pinned.
- **GPU worker image** (CUDA execution provider).
- **OCR**, a dedicated NSFW head, best-shot and burst stacks, video multi-frame.
- **Several primaries per model.** Needs one home, a removal cascade and a rename cascade across
  primaries.
- **Tidying stale manual paths** after a layout change, as an explicit user action.

**Rejected:**

| Rejected | Why |
|:---------|:----|
| Python sidecar, `candle`, `tract` | `ort` covers CPU and GPU with every model |
| Admin-selectable or non-commercial models | breaks federation and paid hosting |
| Corrections in service config | whole-service invalidation, unbounded growth |
| Absorbing manual tags into stored memberships | duplicated truth, and the pipeline writing model state |
| Generic suppressions for every service type | rule tags are fixed at the rule |
| Analyses pushed inside announcements | bigger messages, and a re-announce when analysis lands late |
| A per-share ML consent flag | the recipient can already download and analyse the pixels |
| A separate ML worker image | one image for self-hosters |
| A face-crop thumbnail system | tag covers with a focus box instead |
| A grid fix mode for faces | the Lightbox faces mode pages through any view |
| Every tree's manual tags as decisions | no single owner |
| A model per service | duplicates facts |
| "Outputs" inside one service | loses per-tree pipeline machinery |
| One concepts or timeline model per user | they are choices, so several are allowed |
| Refusing manual tags under read-only trees | they are accepted and ignored |
| Concept / timeline group rule leaves | text queries and tags cover them |
