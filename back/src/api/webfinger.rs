use crate::infra::settings;
use crate::infra::settings::keys;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct WebFingerQuery {
    pub resource: String,
}

#[derive(Serialize)]
struct WebFingerResponse {
    subject: String,
    links: Vec<WebFingerLink>,
}

#[derive(Serialize)]
struct WebFingerLink {
    rel: String,
    href: String,
}

/// Standalone WebFinger endpoint, active when `USE_RESOLVER=false`.
///
/// Parses `archypix:@username:global_domain` (domain may include a port),
/// validates the domain against `GLOBAL_DOMAIN`, and returns this backend's
/// public base URL. Returns 400 for malformed resources, 404 if the domain
/// does not match this instance's `GLOBAL_DOMAIN`.
#[tracing::instrument(skip(state, query), fields(resource = %query.resource))]
pub async fn handler(
    State(state): State<AppState>,
    Query(query): Query<WebFingerQuery>,
) -> Result<Response, AppError> {
    let (username, domain) = parse_acct_resource(&query.resource).ok_or_else(|| {
        AppError::BadRequest("Invalid resource format. Expected archypix:@user:domain".to_string())
    })?;

    if domain != state.settings.get(keys::GLOBAL_DOMAIN) {
        return Err(AppError::NotFound);
    }

    let public_base_url = settings::public_base_url(&state.settings);

    let body = WebFingerResponse {
        subject: format!("archypix:@{}:{}", username, domain),
        links: vec![WebFingerLink {
            rel: "backend_url".to_string(),
            href: public_base_url,
        }],
    };
    let json = serde_json::to_string(&body).expect("WebFingerResponse is always serializable");
    Ok((
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/jrd+json"),
        )],
        json,
    )
        .into_response())
}

/// Parse `archypix:@username:domain` into `(username, domain)`.
/// Uses `splitn(2, ':')` on the part after the username so that a
/// domain containing a port (e.g. `localhost:8003`) is preserved whole.
fn parse_acct_resource(resource: &str) -> Option<(&str, &str)> {
    let rest = resource.strip_prefix("archypix:@")?;
    // splitn(2) → ["username", "domain_or_domain:port"]
    let mut iter = rest.splitn(2, ':');
    let username = iter.next()?;
    let domain = iter.next()?;
    if username.is_empty() || domain.is_empty() {
        return None;
    }
    Some((username, domain))
}
