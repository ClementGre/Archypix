# Trace Sampling & OpenTelemetry Collector

## 1. Overview & goals

[`12_observability_tracing.md`](12_observability_tracing.md) wires the backend and worker to export
OTLP traces directly to Jaeger, with bounded-cardinality operation names, HTTP semconv attributes,
client/server span kinds, and per-binary resource attributes. That is enough to *read* traces; it
is not enough to *run* tracing at production volume.

Two gaps remain, and they are linked — the strongest form of the first depends on the second:

- **Sampling** — decide which traces to keep. Without it, bot/404 noise (the same traffic that
  motivated bounded operation names) is stored and *indexed* at full cost, for near-zero
  diagnostic value.
- **An OpenTelemetry Collector in front of Jaeger** — a separate process that receives OTLP from
  every service, then batches, samples, enriches, and fans out to backends.

Both are **infrastructure/config**, not application code. The services already read the standard
OTel env vars; no business-logic file changes. This feature is **not yet implemented** — it is the
deployment-hardening follow-up to feature 12.

## 2. Why these matter

### 2.1 Tracing is the most expensive signal

Every span is stored *and* indexed for search. Unbounded ingest means paying storage + indexing for
traffic you would never open — exactly the bot/404 traffic feature 12's `<unmatched>` fold already
collapses on the *naming* axis but still emits on the *volume* axis. Sampling is the direct lever on
that cost. *How* you sample decides whether you also throw away the traces worth keeping.

### 2.2 Head vs tail sampling

**Head sampling** — keep/drop decided at trace *start*, propagated via the W3C `traceparent`
sampled flag.

- *Virtue:* **consistency**. Every service in a trace honours the same decision, so you never get a
  half-sampled trace. It is cheap and needs no extra infra — purely the SDK env vars.
- *Flaw:* **blind**. The decision is made before the outcome is known, so a 1% rate drops ~99% of
  your errors too. You cannot express "keep all the broken ones".

**Tail sampling** — buffer all spans of a trace until it completes, *then* decide on what actually
happened: keep 100% of traces with an error or a p99-outlier latency, sample 1% of healthy ones.

- *Virtue:* cuts volume by 1–2 orders of magnitude while losing almost no debugging value — the
  traces it keeps are exactly the ones you would ever open.
- *Cost:* needs a component that sees **all** spans of a trace before deciding. A single backend
  process cannot: a trace spans back → worker → peer backend, across processes. That stateful
  aggregation point is the Collector. **This is why §2.3 is a prerequisite for tail sampling.**

### 2.3 Why a Collector in front of Jaeger

Today each service exports OTLP straight to Jaeger. Inserting a Collector is better for reasons that
compound:

1. **Decoupling / reliability.** The app exports to a local Collector (localhost — fast, basically
   never fails) and moves on; the Collector owns batching, retries, and backpressure to the real
   backend. If Jaeger is down or being redeployed, the Collector buffers — the app neither drops
   spans nor stalls on the export path. Direct export couples Jaeger's availability into every
   service.
2. **It is the home for tail sampling** (§2.2) — the only trace-aware, stateful place that can do
   what head sampling cannot.
3. **SpanMetrics connector — the multiplier on feature 12.** The Collector derives RED metrics
   (request rate, error rate, duration histograms) *per operation* directly from spans, emitted as
   real metrics for dashboards and alerting, with no separate metrics instrumentation. This only
   works cleanly with low-cardinality operation names — precisely what feature 12 established. So
   the Collector turns that naming work into free per-route latency/error dashboards.
4. **Fan-out + central config.** Send the same telemetry to Jaeger + a metrics store + cold storage
   by editing one Collector config, not N services' env vars. Sampling rate, a new backend, a
   redacted attribute — all change with a config reload, no app redeploy. Observability ops are
   decoupled from the release cycle.
5. **Enrichment / scrubbing in one place.** `resourcedetection` adds host/k8s metadata; attribute
   processors drop or scrub sensitive fields centrally, rather than spreading that logic across app
   code (which [`12`](12_observability_tracing.md) §3.5 wants kept minimal).
6. **Supported direction.** Jaeger v2 is itself built on the OTel Collector, so this is where the
   ecosystem already is — not a detour.

## 3. Plan

### Step 1 — Head sampling (no new infra)

Set the SDK env vars feature 12 already reads (`back`/`worker` do not re-parse them):

```
OTEL_TRACES_SAMPLER=parentbased_traceidratio
OTEL_TRACES_SAMPLER_ARG=0.1      # keep 10% of root traces; children inherit via traceparent
```

`parentbased_*` ensures a sampled parent keeps its children (and an unsampled parent drops them), so
cross-process traces (back → worker, back → peer) stay whole. This is shippable immediately and
already cuts volume; its only weakness is that it is blind to errors (§2.2).

### Step 2 — Collector + tail sampling

1. Add an `otel/opentelemetry-collector-contrib` service to the dev compose (the `contrib`
   distribution ships the `tail_sampling` and `spanmetrics` components).
2. Point every service's `OTEL_EXPORTER_OTLP_ENDPOINT` at the Collector instead of Jaeger; switch
   the SDK sampler to `parentbased_always_on` (sample everything at the edge — the Collector makes
   the real decision with full-trace context).
3. Collector pipeline:
    - `receivers: [otlp]`
    - `processors: [tail_sampling, resourcedetection, batch]` — tail policy: keep on error
      (`status_code == ERROR`) OR latency over threshold, else probabilistic ~1–5%.
    - `connectors: [spanmetrics]` — derive RED metrics from spans, keyed by the bounded operation
      name.
    - `exporters:` Jaeger (OTLP) for traces; a metrics backend for the SpanMetrics output.

Tail sampling supersedes Step 1's head sampling — switch the edge sampler to always-on once the
Collector is in the path, so the tail policy sees every span.

## 4. Decisions (settled)

- **Head sampling first, tail sampling once the Collector lands.** Head sampling needs zero infra
  and is a useful interim; do not block volume control on the Collector.
- **No in-process tail sampling.** It is structurally impossible (no single process sees a full
  cross-service trace) — do not attempt an app-side approximation.
- **Operation-name cardinality is a hard prerequisite** for the SpanMetrics connector. It is already
  satisfied by feature 12; any new span must keep `otel.name` low-cardinality (see
  [`12`](12_observability_tracing.md) §3.2 "Operation naming & cardinality").

## 5. Configuration

| Env var                       | Where       | Meaning                                                                |
|-------------------------------|-------------|------------------------------------------------------------------------|
| `OTEL_TRACES_SAMPLER`         | back/worker | `parentbased_traceidratio` (Step 1) → `parentbased_always_on` (Step 2) |
| `OTEL_TRACES_SAMPLER_ARG`     | back/worker | Step-1 keep ratio (e.g. `0.1`)                                         |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | back/worker | Jaeger directly (feature 12) → Collector (Step 2)                      |

All read by the OTel SDK; not re-parsed in app code.
