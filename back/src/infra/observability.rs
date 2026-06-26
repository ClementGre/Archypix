use opentelemetry::propagation::{Extractor, Injector};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::{Tracer, TracerProvider};
use std::collections::HashMap;
use tracing_subscriber::prelude::*;

pub struct ObservabilityGuard {
    _provider: Option<TracerProvider>,
}

impl Drop for ObservabilityGuard {
    fn drop(&mut self) {
        if let Some(provider) = self._provider.take() {
            if let Err(e) = provider.shutdown() {
                eprintln!("OTLP shutdown error: {e:?}");
            }
        }
    }
}

/// Initialise the global tracing subscriber. The returned guard must be kept alive for the
/// process lifetime; on drop it flushes and shuts down the OTLP exporter (Step 2 only).
pub fn init(service_name: &'static str, instance_id: String) -> ObservabilityGuard {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,archypix_back=debug".into());
    let use_json = matches!(std::env::var("LOG_FORMAT").as_deref(), Ok("json"));
    let (provider, tracer_opt) = build_otel_tracer(service_name, instance_id);

    // OTel layer must NOT be boxed: its `S` type parameter is generic over the subscriber
    // and must be inferred at the `.with()` call site. Boxing it to `dyn Layer<Registry>`
    // would lock `S = Registry`, making it incompatible with the already-layered subscriber.
    let base = tracing_subscriber::registry().with(env_filter);
    if use_json {
        let sub = base.with(tracing_subscriber::fmt::layer().json());
        if let Some(t) = tracer_opt {
            sub.with(tracing_opentelemetry::layer().with_tracer(t))
                .init();
        } else {
            sub.init();
        }
    } else {
        let sub = base.with(tracing_subscriber::fmt::layer());
        if let Some(t) = tracer_opt {
            sub.with(tracing_opentelemetry::layer().with_tracer(t))
                .init();
        } else {
            sub.init();
        }
    }

    ObservabilityGuard {
        _provider: provider,
    }
}

fn build_otel_tracer(
    service_name: &'static str,
    instance_id: String,
) -> (Option<TracerProvider>, Option<Tracer>) {
    if std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err() {
        return (None, None);
    }

    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to build OTLP exporter: {e:?}");
            return (None, None);
        }
    };

    // `deployment.environment` lets Jaeger separate dev/staging/prod traces; `service.version`
    // (the crate version, resolved per-binary at compile time) ties a regression to a release.
    // Both are standard OTel resource attributes.
    let environment =
        std::env::var("DEPLOYMENT_ENVIRONMENT").unwrap_or_else(|_| "development".to_string());
    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(opentelemetry_sdk::Resource::new([
            opentelemetry::KeyValue::new("service.name", service_name),
            opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
            opentelemetry::KeyValue::new("deployment.environment", environment),
            opentelemetry::KeyValue::new("instance.domain", instance_id),
        ]))
        .build();

    let tracer = provider.tracer(service_name);
    (Some(provider), Some(tracer))
}

// ── Carrier helpers ───────────────────────────────────────────────────────────

struct HashMapInjector<'a>(&'a mut HashMap<String, String>);
struct HashMapExtractor<'a>(&'a HashMap<String, String>);

impl Injector for HashMapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

impl Extractor for HashMapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

/// Inject the current tracing span's OTel context into a `HashMap` carrier.
/// Returns an empty map when OTel is disabled (no propagator registered).
pub fn inject_context() -> HashMap<String, String> {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let mut map = HashMap::new();
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|p| {
        p.inject_context(&cx, &mut HashMapInjector(&mut map));
    });
    map
}

/// Extract an OTel `Context` from a `HashMap` carrier (e.g. from the DB or request headers).
pub fn extract_context(map: &HashMap<String, String>) -> opentelemetry::Context {
    opentelemetry::global::get_text_map_propagator(|p| p.extract(&HashMapExtractor(map)))
}

/// Inject the current context into HTTP request headers (reqwest).
pub fn inject_into_headers(headers: &mut reqwest::header::HeaderMap) {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    let mut map = HashMap::new();
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|p| {
        p.inject_context(&cx, &mut HashMapInjector(&mut map));
    });
    for (k, v) in map {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(&v),
        ) {
            headers.insert(name, val);
        }
    }
}

/// Extract an OTel `Context` from HTTP request headers (axum `HeaderMap`).
pub fn extract_from_headers(headers: &axum::http::HeaderMap) -> opentelemetry::Context {
    let map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            let val = v.to_str().ok()?.to_owned();
            Some((k.as_str().to_owned(), val))
        })
        .collect();
    opentelemetry::global::get_text_map_propagator(|p| p.extract(&HashMapExtractor(&map)))
}

/// Reparent the current span to the remote trace iff the *authenticated* peer is allow-listed.
///
/// Gating on the JWT-verified identity (not a raw header) prevents a hostile instance from
/// spoofing a trusted peer to get its trace headers honoured.
pub fn maybe_set_remote_parent(
    headers: &http::HeaderMap,
    peer_global_domain: &str,
    cfg: &crate::infra::config::Config,
) {
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    if !cfg
        .trace_propagation_peers
        .iter()
        .any(|p| p == peer_global_domain)
    {
        return;
    }
    let cx = extract_from_headers(headers);
    tracing::Span::current().set_parent(cx);
}
