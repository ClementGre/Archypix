# Observability: Structured Tracing & OpenTelemetry

## 1. Overview & goals

The backend and worker currently log with a plain `tracing_subscriber::fmt()` subscriber to
stdout, with no spans and no request correlation
([`back/src/main.rs:22`](../../back/src/main.rs), [`worker/src/main.rs:16`](../../worker/src/main.rs)).
Concurrent requests interleave their log lines on stdout with nothing to tie them together —
the "logs mixed up by multi-threading" problem.

This feature is delivered in **two independent, sequenced steps**:

- **Step 1 — Structured, span-correlated logs (no external backend).** Add a per-request root
  span carrying a `request_id`, and `#[instrument]` the workflow layer so every log line knows
  which request/job/user it belongs to. Pure `tracing`; opens no ports, runs no extra infra.
  This alone fixes the multi-threading log soup and is independently shippable.
- **Step 2 — OpenTelemetry (OTLP) export to Jaeger.** Add an OpenTelemetry layer to the *same*
  subscriber so the spans created in Step 1 are *also* exported as distributed traces, plus the
  cross-process propagation glue to stitch them into one trace per request.

**Step 2 reuses Step 1 verbatim.** The `tracing` subscriber fans the same span/event stream out
to every registered layer. Step 1 registers a `fmt` (stdout) layer; Step 2 adds an
`opentelemetry` layer to the same registry. No business-logic file is re-instrumented for
Step 2 — adding more `#[instrument]` later enriches *both* stdout and Jaeger. The only code
Step 2 adds beyond the subscriber wiring is the cross-process propagation at the boundaries.

### Hops in scope (Step 2)

| Hop                            | Trace handling                                                                                                                                                                        |
|--------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| backend → worker (job queue)   | **Always propagated**, via a `traceparent` stored on the job row + **span link** on the worker side; plus parent-child on the worker→backend `complete`/`fail` call. The main payoff. |
| backend → backend (federation) | **Allowlist-gated**, off by default. Propagated only to/from peers explicitly configured as trusted (same operator, same Jaeger). Otherwise a fresh root span.                        |

### Hops explicitly OUT of scope

- **frontend → backend.** The frontend is a static SPA with no server side; participating would
  require shipping an OTel JS SDK and a browser→collector export path (RUM) — a separable
  concern. The backend mints the root span at HTTP ingress, so the *backend-side* trace is
  complete without it. We instead echo the generated `request_id` back as the `x-request-id`
  response header for cheap support-style correlation (§3.2).
- **In-process pipeline wake (`PipelineWaker` mpsc).** A worker completion re-dirties a picture
  and wakes the pipeline through an in-memory channel; carrying trace context across that channel
  is a future extension, not part of this feature. The worker→backend completion *handler* is
  traced; the asynchronous pipeline run it triggers starts its own root span.

### Why federation usefulness == federation safety

Federation connects instances owned by **different people** (a trust boundary; auth is pairwise
JWT). Propagating trace context across it is only *useful* when both backends export to the
**same** Jaeger (otherwise the trace id is meaningless on the other side — you cannot query a
peer's Jaeger). That is exactly the case where it is also *safe*: same operator. For an
untrusted foreign instance it is neither useful nor safe. One allowlist therefore answers both
questions — see §4.5.

The `traceparent` header itself (`00-<32hex traceid>-<16hex spanid>-<2hex flags>`) carries only
opaque random ids and a sampling bit — no usernames, paths, or payload. The real leak risk is
**span attributes** (§3.5), which is internal (who can read your Jaeger) and applies even to a
single instance.

## 2. Decisions (settled)

- **One subscriber, layered.** A single `tracing_subscriber::registry()` with an `EnvFilter`, a
  `fmt` layer (always), and — when enabled — an `opentelemetry` layer. Same construction in
  `back` and `worker`, each in a new `observability` module.
- **Step 2 is enabled by the presence of `OTEL_EXPORTER_OTLP_ENDPOINT`.** Unset ⇒ Step 1 only
  (fmt to stdout). No code path differs between dev and prod beyond this env var.
- **OTLP over HTTP/protobuf** (port 4318), reusing the `reqwest` stack already in both crates —
  no gRPC/tonic dependency. Modern Jaeger ingests OTLP natively; the deprecated
  `opentelemetry-jaeger` crate is **not** used.
- **backend → worker is always propagated** (inside the trust boundary; the worker is the
  operator's own process). The job queue is an async boundary, so the worker uses a **span link**
  to the enqueueing context rather than parent-child (the enqueuing request has usually ended).
- **federation propagation is allowlist-gated and off by default**, keyed on the *authenticated*
  peer global domain (from the verified federation JWT, never a client-supplied header) so it
  cannot be spoofed.
- **`archypix-common` stays OTel-free.** The trace context on the wire is a plain
  `HashMap<String, String>` (W3C carrier); no OTel types cross into the shared crate.
- **Field/attribute hygiene is a hard rule** (§3.5): never record presigned URLs, JWTs, tokens,
  passwords, or full EXIF/metadata blobs as span fields or attributes.

## 3. Step 1 — Structured, span-correlated logs

### 3.1 Subscriber construction

Add `back/src/infra/observability.rs` (register `pub mod observability;` in `back/src/infra.rs`;
repo convention is a `observability.rs` file, **no** `mod.rs`) and the mirror
`worker/src/observability.rs`. Each exposes:

```rust
/// Initialise the global tracing subscriber. Returns a guard that must be kept alive for the
/// process lifetime; on drop (Step 2 only) it flushes and shuts down the OTLP exporter.
pub fn init(/* service name, resource attrs, config */) -> ObservabilityGuard { … }
```

Step-1 body (back; worker identical apart from the default filter directive and service name):

```rust
use tracing_subscriber::prelude::*;

let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
.unwrap_or_else( | _ | "info,archypix_back=debug".into());

let fmt_layer = match std::env::var("LOG_FORMAT").as_deref() {
Ok("json") => tracing_subscriber::fmt::layer().json().boxed(),
_ => tracing_subscriber::fmt::layer().boxed(),
};

tracing_subscriber::registry()
.with(env_filter)
.with(fmt_layer)
// Step 2 adds `.with(otel_layer)` here — see §4.2.
.init();
```

Replace the existing `tracing_subscriber::fmt()…init()` blocks in both `main.rs` files with a
call to this `init()`. In `back`, call it **after** `Config::from_env()` so resource attributes
(global domain) are available in Step 2; keep the early startup `info!` lines after the call.

`tracing-subscriber` already has the `env-filter` feature; `registry`, `fmt`, and `json` are
covered by enabling the `json` feature. Update both `Cargo.toml`:
`tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }`.

### 3.2 Per-request root span + request id (backend only)

The backend already compiles `tower-http`'s `request-id` feature but does not use it
([`back/Cargo.toml:11`](../../back/Cargo.toml)). Wire the request-id layers around the existing
`TraceLayer` ([`back/src/main.rs:147`](../../back/src/main.rs)) so each request gets a stable id
that appears in its span **and** is echoed to the client:

```rust
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

const REQUEST_ID: http::HeaderName = http::HeaderName::from_static("x-request-id");

// outermost → innermost
.layer(PropagateRequestIdLayer::new(REQUEST_ID.clone()))     // copy id onto the response
.layer(
TraceLayer::new_for_http().make_span_with( | req: & http::Request<_ > | {
let request_id = req
.headers()
.get("x-request-id")
.and_then( | v | v.to_str().ok())
.unwrap_or("unknown")
.to_owned();
// The Jaeger operation name (`otel.name`) is the *matched route template*, not the concrete
// URI path — see "Operation naming & cardinality" below.
let route = req.extensions().get::< axum::extract::MatchedPath > ().map( | m | m.as_str());
let otel_name = match route {
Some(route) => format ! ("{} {}", req.method(), route),
None => format ! ("{} <unmatched>", req.method()),
};
// Field names follow the OTel HTTP semantic conventions (semconv) so Jaeger and the SpanMetrics
// connector consume them without per-tool remapping. `otel.kind = "server"` marks the inbound span.
// `client.address` (peer IP from `ConnectInfo`) attributes a burst of `<unmatched>` 404s to one
// scanning source; the concrete path stays searchable via `url.path`.
tracing::info_span ! (
"http_request",
"otel.name" = otel_name,
"otel.kind" = "server",
"http.request.method" = % req.method(),
"http.route" = route.unwrap_or("<unmatched>"),
"url.path" = % req.uri().path(),
"client.address" = % client_addr,
"server.address" = % server_addr,
request_id = % request_id,
// reserved, filled later: status by `on_response`, enduser.id by auth middleware:
"http.response.status_code" = tracing::field::Empty,
"otel.status_code" = tracing::field::Empty,
"enduser.id" = tracing::field::Empty,
)
})
// `on_response` stamps the status and sets the OTel span status to ERROR on 5xx so Jaeger flags
// failed traces (4xx stay Unset — a client error is not a server fault).
.on_response(record_response),
)
.layer(SetRequestIdLayer::new(REQUEST_ID.clone(), MakeRequestUuid)) // generate id first
```

Order matters: `SetRequestIdLayer` must run **before** `TraceLayer` so the header exists when
`make_span_with` reads it. Auth middleware that resolves the user records it onto the current span:
`tracing::Span::current().record("enduser.id", tracing::field::display(uid));`.

> **Field-name syntax:** `info_span!` accepts quoted dotted names (`"otel.kind" = …`);
> `#[tracing::instrument(fields(…))]` does **not** — there the same fields are bare dotted idents
> (`otel.kind = "client"`). Both forms produce identical OTel attributes.

#### Operation naming & cardinality

`otel.name` becomes the Jaeger **operation name** — the primary axis Jaeger groups traces by. It
must be **low-cardinality**, so it is derived from the matched route, never the raw path:

- The concrete path (`/api/authenticated/pictures/3f8c…`, WebDAV URIs) would mint a *new* operation
  per id, and 404-scanning bots would mint one per random path they probe — quickly polluting the
  operation list and Jaeger's index.
- `MatchedPath` (populated by axum's router, available here because the layer wraps the routed
  `Router`) collapses every parametrised request to its template (`/api/authenticated/pictures/{id}`).
  Requests that match no route (bot 404s, unknown paths) carry no `MatchedPath`, so they all fold
  into a single `{METHOD} <unmatched>` operation. The concrete path is still searchable via the
  `path` span attribute.

The worker applies the same principle: its job root span sets `otel.name = job_type`
(`gen_thumbnail` / `edit_picture`), grouping by kind of work rather than collapsing all jobs into a
generic `job` operation.

### 3.3 Instrumenting the workflow layer

Annotate the multi-step workflow functions so the trace tree mirrors the call graph. Apply
`#[tracing::instrument]` to:

- All public functions in `back/src/services/*` (share lifecycle, pipeline evaluation entry,
  hierarchy/vfs/webdav resolution, picture/upload orchestration).
- The pipeline run entry and the federation inbound handlers in `back/src/api/federation/`.
- The worker job entry and imaging steps (§3.4).

Conventions:

```rust
#[tracing::instrument(
    skip(state, db),                       // never log big/secret args (§3.5)
    fields(user_id = %user.id, picture_id = %picture_id)
)]
pub async fn edit_picture_exif(…) -> Result<…> { … }
```

- **Always `skip`** `AppState`, pool/transaction handles, request bodies, byte buffers, and any
  struct that may contain secrets; add back only the safe scalar ids via `fields(...)`.
- Record ids you have: `user_id`, `picture_id`, `share_id`, `job_id`, `owner` (global domain).
- **Async footgun:** never hold a manual `span.enter()` guard across an `.await`. Use
  `#[instrument]` (async-aware) or `future.instrument(span)`; both correctly re-enter the span
  when Tokio resumes the task on any thread. This is the mechanism that makes correlation
  independent of stdout interleaving.

### 3.4 Background tasks / spawned work

Spawned loops have no inbound request, so they must mint their own root span:

- `infra/scheduler.rs` recurring ticks: wrap each tick in `info_span!("recurring_task", task =
  %self.name(), run_id = %Uuid::new_v4())`.
- `infra/pipeline.rs` per-user run: `info_span!("pipeline_run", user_id = %uid, run_id = …)`.
- `infra/tasks.rs` `TaskQueue` items: a span per task with the task kind + ids.
- Worker `dispatch(job)` ([`worker/src/jobs.rs:64`](../../worker/src/jobs.rs)):
  `info_span!("job", job_id = %job.job_id, job_type = ?job.job_type, picture_id = ?job.picture_id)`,
  entered for the whole job; the poll loop itself stays unspanned (or a tiny `trace!`-level span)
  to avoid noise.

### 3.5 Field & attribute hygiene (hard rule)

This applies to both the fmt layer (Step 1) and the OTLP layer (Step 2). **Never** put the
following in a span field, `record(...)`, or event:

- Presigned S3 URLs (they embed signatures/tokens) — log the S3 key or picture id instead.
- JWTs / bearer tokens / `claim_token` / federation tokens / passwords / password hashes.
- Full EXIF blobs, `local_exif_overrides`, file bytes, or whole request/response bodies.

Log identifiers and outcomes, not payloads. When in doubt, log the id and a boolean/length.

### 3.6 JSON output toggle

`LOG_FORMAT=json` switches the fmt layer to structured JSON (machine-grep by `request_id`);
anything else keeps the human-readable formatter. Default human.

## 4. Step 2 — OpenTelemetry export to Jaeger

### 4.1 Dependencies

Add to **both** `back/Cargo.toml` and `worker/Cargo.toml` (verify mutually compatible versions
at implementation time — the set below is internally consistent):

```toml
opentelemetry = "0.27"
opentelemetry_sdk = { version = "0.27", features = ["rt-tokio"] }
opentelemetry-otlp = { version = "0.27", features = ["http-proto", "reqwest-client"] }
tracing-opentelemetry = "0.28"
```

`archypix-common` gets **no** new dependency (§2).

### 4.2 Subscriber: adding the OTLP layer

In `observability::init`, build the layer only when `OTEL_EXPORTER_OTLP_ENDPOINT` is set:

```rust
let otel_layer = if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok() {
opentelemetry::global::set_text_map_propagator(
opentelemetry_sdk::propagation::TraceContextPropagator::new()); // W3C traceparent

let exporter = opentelemetry_otlp::SpanExporter::builder()
.with_http()                       // endpoint + sampler come from OTEL_* env vars
.build() ?;

let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
.with_batch_exporter(exporter)
.with_resource(
opentelemetry_sdk::Resource::builder()
.with_attributes([
opentelemetry::KeyValue::new("service.name", service_name),       // archypix-back | archypix-worker
opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")), // per-binary crate version
opentelemetry::KeyValue::new("deployment.environment", environment),       // DEPLOYMENT_ENVIRONMENT, default "development"
opentelemetry::KeyValue::new("instance.domain", global_domain),   // back: config.global_domain; worker: worker_id
])
.build())
.build();

let tracer = provider.tracer(service_name);
Some((provider, tracing_opentelemetry::layer().with_tracer(tracer)))
} else { None };
```

Add `.with(otel_layer)` to the registry from §3.1 (use `Option`'s `Layer` impl, or `.boxed()`).

- **Resource attributes** distinguish instances in Jaeger: `service.name` per crate, plus
  `instance.domain` = the backend's global domain (so `backend1`/`backend2` traces are
  separable). For the worker use `worker_id`. `service.version` (the crate version) and
  `deployment.environment` (`DEPLOYMENT_ENVIRONMENT`, default `development`) let you filter
  dev/staging/prod and tie a regression to a release.
- **Service granularity** — one `service.name` per *deployable process* (`archypix-back`,
  `archypix-worker`, `archypix-resolver`), never one per code layer. `service.name` drives Jaeger's
  service list and dependency graph, so splitting `repository`/`services`/`infra` into services
  would fragment a single request's trace and pollute the graph with intra-process edges. Slice by
  layer with span attributes / tracer (scope) names instead.
- **Client spans** — outbound calls are marked `otel.kind = "client"`: every federation HTTP call
  (`clients/federation/*`, `clients/resolver.rs`) and every *networked* S3 op (`infra/s3.rs`
  get/put/head/copy/delete — not the local `presign_*` signing). S3 ops also carry
  `peer.service = "s3"` so Jaeger draws an edge to MinIO even though it emits no spans of its own.
  Federation calls need no `peer.service`: the peer backend is itself instrumented and (for trusted
  peers, §4.5) continues the same trace.
- **Shutdown flush:** the batch exporter buffers spans; return the `provider` inside
  `ObservabilityGuard` and call `provider.shutdown()` on drop / before `main` returns, else the
  last spans are lost. Both `main.rs` must hold the guard for the whole process.
- Endpoint, sampler ratio, timeout are standard OTel env vars
  (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_TRACES_SAMPLER`, `OTEL_TRACES_SAMPLER_ARG`) read by the
  SDK — do not re-parse them in app code.

### 4.3 Context propagation model

Propagation uses the W3C `TraceContextPropagator` set globally in §4.2. The bridge between
`tracing` spans and OTel context is `tracing_opentelemetry::OpenTelemetrySpanExt`:

- **Read current context** (to inject outbound): `tracing::Span::current().context()`.
- **Set a remote parent** (inbound, trusted): `span.set_parent(remote_cx)`.
- **Add a link** (inbound, decoupled queue): `span.add_link(remote_span_context)`.

Carrier helpers (one small module, e.g. `infra/observability.rs` in back, `observability.rs` in
worker): inject the current context into a `HashMap<String,String>` and extract a `Context` from
one, via `opentelemetry::global::get_text_map_propagator(|p| …)` with a `HashMap` injector/
extractor (`opentelemetry::propagation::{Injector, Extractor}`). When Step 2 is disabled the
current context is empty, so inject yields an empty map → treat as "no context".

### 4.4 Hop A — backend → worker (job queue)

The enqueuing request and the worker that runs the job are decoupled in time, so use a **span
link**, not parent-child.

**Schema.** Add a nullable `trace_context JSONB` column to the `jobs` table. Per the coding
guidelines (`doc/00_CODING_GUIDELINES.md` §"Database migrations"), edit the single
`back/migrations/001_initial_schema.up.sql` (add the column inside the `CREATE TABLE jobs` block,
~line 569, next to `config`; add a matching line to `001_initial_schema.down.sql` if it drops
columns explicitly), then run the rebuild + `cargo sqlx prepare` workflow, add an idempotent
`ALTER TABLE jobs ADD COLUMN IF NOT EXISTS trace_context JSONB;` to a `docker/migrations/`
script for the seeded test DBs, and run `docker/migrations/fix_migration_checksum.sh`.

```sql
-- W3C trace context (traceparent/tracestate) captured when the job was enqueued, so the worker
-- can link its processing span to the originating trace. NULL when tracing is disabled.
trace_context JSONB,
```

**Enqueue (capture).** Where jobs are inserted
([`back/src/repository/job.rs:223`](../../back/src/repository/job.rs)), capture the current
context into the W3C carrier map and store it (NULL when empty). Add `trace_context` to the
`INSERT INTO jobs (...)` column list and bind the optional JSON.

**Claim (carry to worker).** Add a field to the wire type
([`common/src/transfer.rs:37`](../../common/src/transfer.rs)):

```rust
/// W3C trace context captured at enqueue time; the worker links its job span to it.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub trace_context: Option<std::collections::HashMap<String, String> >,
```

Populate it in both `ClaimJobResponse` constructions in
[`back/src/api/worker/handlers.rs`](../../back/src/api/worker/handlers.rs) (lines ~53 and ~175)
from the claimed job row.

**Worker (link).** In `dispatch(job)` ([`worker/src/jobs.rs:64`](../../worker/src/jobs.rs)),
after creating the `info_span!("job", …)` from §3.4, if `job.trace_context` is present extract a
`Context` from it and `job_span.add_link(remote_cx.span().span_context().clone())`. The job span
is a **new root** in the worker's trace, linked back to the enqueuing trace — bounded, and
correct for the async boundary.

**Completion (parent-child).** The worker→backend `complete`/`fail` calls *are* synchronous, so
trace them parent-child:

- Worker: in `BackendClient::complete_job` / `fail_job`
  ([`worker/src/backend.rs`](../../worker/src/backend.rs)), inject the current (job span) context
  into the outbound request headers.
- Backend: the worker completion handlers
  ([`back/src/api/worker/handlers.rs`](../../back/src/api/worker/handlers.rs)) extract the headers
  and `Span::current().set_parent(cx)` at the top. The worker JWT already authenticates these as
  trusted same-operator calls, so no allowlist gating is needed here.

This yields one connected trace: `enqueue (link) → worker job span → backend completion handler`.
The pipeline re-announce it triggers via `PipelineWaker` starts its own root (out of scope, §1).

### 4.5 Hop B — backend → backend (federation), allowlist-gated

**Config.** Add `trace_propagation_peers: Vec<String>` to
[`back/src/infra/config.rs`](../../back/src/infra/config.rs) `Config` (parse a comma-separated
`TRACE_PROPAGATION_PEERS` env var of **global domains**; default empty). These are the peers that
share this operator's Jaeger.

**Outbound (inject).** In the federation client
([`back/src/clients/federation/shares.rs`](../../back/src/clients/federation/shares.rs) and
`handshake.rs`), before each `.post(&url)` to a peer, inject the current context into the request
headers **only if** the recipient's global domain ∈ `trace_propagation_peers`. Otherwise send no
trace headers (the peer starts a fresh root). Thread the recipient global domain + allowlist to
these call sites (they already know the recipient identity).

**Inbound (extract).** Federation handlers receive an `AuthFederation` extractor whose verified
peer global domain comes from the federation JWT
([`back/src/api/middleware/auth_federation.rs`](../../back/src/api/middleware/auth_federation.rs)).
Add a helper and call it at the top of each federation handler to be traced:

```rust
/// Reparent the current span to the remote trace iff the *authenticated* peer is allow-listed.
/// Gating on the JWT-verified identity (not a header) prevents a hostile instance from
/// spoofing a trusted peer to get its trace headers honoured.
pub fn maybe_set_remote_parent(headers: &HeaderMap, peer: &str, cfg: &Config) {
    if !cfg.trace_propagation_peers.iter().any(|p| p == peer) { return; }
    let cx = extract_context(headers);          // §4.3 extractor
    tracing::Span::current().set_parent(cx);
}
```

For a non-allowlisted peer the headers are **not even read** — no remote sampling flag is
trusted, no `tracestate` is parsed, and the request is traced as a local root. (Within the
allowlist, peers are same-operator so honouring their parent + sampling is acceptable.)

### 4.6 Out-of-scope hops (recap)

- **frontend → backend:** not propagated; backend mints the root, echoes `x-request-id` (§3.2).
- **`PipelineWaker` mpsc:** the in-process wake after worker completion is not propagated; the
  pipeline run is its own root span.

## 5. Configuration

| Env var (service)                            | Default                 | Meaning                                                                         |
|----------------------------------------------|-------------------------|---------------------------------------------------------------------------------|
| `RUST_LOG` (back, worker)                    | `info,archypix_*=debug` | Standard `EnvFilter` directive.                                                 |
| `LOG_FORMAT` (back, worker)                  | human                   | `json` ⇒ structured JSON logs (Step 1).                                         |
| `OTEL_EXPORTER_OTLP_ENDPOINT` (back, worker) | unset                   | **Master switch for Step 2.** e.g. `http://jaeger:4318`. Unset ⇒ Step 1 only.   |
| `OTEL_TRACES_SAMPLER` / `_ARG`               | SDK default             | Sampling strategy/ratio (read by the SDK).                                      |
| `TRACE_PROPAGATION_PEERS` (back)             | empty                   | Comma-separated global domains trusted for federation trace propagation (§4.5). |

## 6. Jaeger deployment (dev)

Add an all-in-one Jaeger service to the dev compose; it ingests OTLP directly:

```yaml
jaeger:
  image: jaegertracing/all-in-one:latest
  environment:
    COLLECTOR_OTLP_ENABLED: "true"
  ports:
    - "16686:16686"   # UI
    - "4318:4318"     # OTLP/HTTP (matches OTEL_EXPORTER_OTLP_ENDPOINT)
```

Set `OTEL_EXPORTER_OTLP_ENDPOINT=http://jaeger:4318` on `back` and `worker`; open the UI at
`http://localhost:16686`. For multiple same-operator backends, point them all at this one Jaeger
and list each other in `TRACE_PROPAGATION_PEERS` to see federation traces joined.

## 7. Files touched (checklist)

**Step 1**

- `back/src/infra/observability.rs` (new) + register in `back/src/infra.rs`.
- `worker/src/observability.rs` (new) + register in `worker/src/main.rs` module list.
- `back/src/main.rs` — call `observability::init`; wire request-id + `make_span_with` around
  `TraceLayer`.
- `worker/src/main.rs` — call `observability::init`.
- `back/Cargo.toml`, `worker/Cargo.toml` — `tracing-subscriber` `json` feature.
- `#[instrument]` across `back/src/services/*`, `api/federation/*`, `infra/{pipeline,scheduler,
  tasks}.rs`, `worker/src/jobs.rs` + `imaging/*`.
- Auth middleware records `user_id` onto the current span.

**Step 2** (everything above unchanged, plus)

- `back/Cargo.toml`, `worker/Cargo.toml` — OTel deps (§4.1).
- `observability.rs` (both) — OTLP layer, propagator, resource, shutdown guard, carrier helpers.
- `back/migrations/001_initial_schema.up.sql` (+ down) — `jobs.trace_context`; `docker/migrations/`
  idempotent script + checksum fix (per coding guidelines).
- `common/src/transfer.rs` — `ClaimJobResponse.trace_context`.
- `back/src/repository/job.rs` — capture/store context on enqueue.
- `back/src/api/worker/handlers.rs` — populate `trace_context` on claim; `set_parent` on
  complete/fail.
- `worker/src/jobs.rs` — `add_link` from job span; `worker/src/backend.rs` — inject context on
  complete/fail.
- `back/src/clients/federation/{shares,handshake}.rs` — allowlist-gated inject.
- `back/src/api/federation/handlers.rs` — `maybe_set_remote_parent` at handler tops.
- `back/src/infra/config.rs` — `trace_propagation_peers`.
- dev compose — Jaeger service.

## 8. Testing

- **Step 1:** a unit test asserting `observability::init` is idempotent/total-order-safe is
  low-value; instead assert the request span carries `request_id` and that `x-request-id` is
  echoed on a response (extend an existing API integration test). Confirm `cargo build` /
  `cargo check --tests` for both crates.
- **Step 2 (no live Jaeger needed):**
    - Round-trip the carrier helpers: inject → extract yields a context with the same trace id.
    - Enqueue→claim: a job inserted while a span is active has a non-NULL `trace_context`, and the
      claimed `ClaimJobResponse` carries it; with tracing disabled it is `None`.
    - Federation gating: `maybe_set_remote_parent` is a no-op for a peer not in
      `trace_propagation_peers` and reparents for one that is (unit test with a fake `HeaderMap`).
- Per coding guidelines, keep the back test suite updated for the new column/wire field.

## 9. Roadmap

Implements the "Logging robustness — Better tracing, logs that does not mix up with
multi-threading (Otel compatibility?)" item in `doc/99_ROADMAP_MVP.md`. Step 1 satisfies the
multi-threading-correlation half on its own; Step 2 adds the OTel/Jaeger half. Mark the roadmap
item and update `doc/03_BACKEND_ARCHITECTURE.md` (new `infra/observability.rs`) and
`doc/04_WORKER_ARCHITECTURE.md` (new `observability.rs` + `trace_context` on the job wire type)
when implemented.

```
