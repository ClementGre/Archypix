//! WebFinger: `@user:global_domain` → owning backend public URL (RFC 7033 JRD).

use crate::repository;
use crate::state::AppState;
use archypix_common::error::AppError;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
pub struct WebFingerQuery {
    resource: String,
}

#[derive(Debug, Serialize)]
struct WebFingerResponse {
    subject: String,
    links: Vec<WebFingerLink>,
}
#[derive(Debug, Serialize)]
struct WebFingerLink {
    rel: String,
    href: String,
}

pub async fn handler(
    Query(query): Query<WebFingerQuery>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let global_domain = state.global_domain();
    let username = parse_acct_resource(&query.resource, &global_domain)?;

    let backend_url = match state.cache.get(&username).await {
        Some(url) => url,
        None => {
            let url = repository::get_backend_url(&state.db, &username)
                .await?
                .ok_or_else(|| {
                    warn!(user = %username, "webfinger: username not found");
                    AppError::NotFound
                })?;
            state.cache.insert(username.clone(), url.clone()).await;
            url
        }
    };
    debug!(user = %username, "webfinger");
    Ok(jrd(WebFingerResponse {
        subject: format!("archypix:@{username}:{global_domain}"),
        links: vec![WebFingerLink {
            rel: "backend_url".to_string(),
            href: backend_url,
        }],
    }))
}

fn jrd(body: WebFingerResponse) -> Response {
    let json = serde_json::to_string(&body).expect("serializable");
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/jrd+json"),
        )],
        json,
    )
        .into_response()
}

fn parse_acct_resource(resource: &str, global_domain: &str) -> Result<String, AppError> {
    let rest = resource
        .strip_prefix("archypix:@")
        .ok_or_else(|| AppError::BadRequest("Expected archypix:@user:domain".to_string()))?;
    // splitn(2) keeps a host:port domain intact.
    let mut it = rest.splitn(2, ':');
    let user = it.next().filter(|u| !u.is_empty());
    let domain = it.next();
    match (user, domain) {
        (Some(user), Some(domain)) if domain == global_domain => Ok(user.to_string()),
        (_, Some(domain)) => Err(AppError::BadRequest(format!("Invalid domain: {domain}"))),
        _ => Err(AppError::BadRequest(
            "Expected archypix:@user:domain".to_string(),
        )),
    }
}
