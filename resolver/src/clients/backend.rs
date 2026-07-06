//! Outbound resolver→backend client (feature 23 §3.2 delegation replay, §5.3 proxy).
//!
//! Every call replays the backend-signed `ResolverDelegation` token the backend delivered via its
//! last heartbeat — the resolver never mints a token a backend will accept. A backend with no live
//! token (never/ stale heartbeat) is unreachable and yields `503`.

use crate::repository;
use archypix_common::error::AppError;
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

#[derive(Clone)]
pub struct BackendClient {
    db: PgPool,
    http: reqwest::Client,
}

impl BackendClient {
    pub fn new(db: PgPool, http: reqwest::Client) -> Self {
        Self { db, http }
    }

    /// Resolve a reachable backend's `(internal_url, delegation_token)`, or `503` if unreachable.
    async fn target(&self, back_domain: &str) -> Result<(String, String), AppError> {
        let b = repository::get_backend(&self.db, back_domain)
            .await?
            .ok_or_else(|| AppError::NotFound)?;
        match (b.reachable, b.delegation_token) {
            (true, Some(token)) => Ok((b.internal_url, token)),
            _ => Err(AppError::ServiceUnavailable(format!(
                "backend '{back_domain}' is unreachable (no live delegation token)"
            ))),
        }
    }

    /// Send a JSON request to `path` on a backend, replaying its delegation token. Returns the parsed
    /// JSON body on 2xx, or a `BackendError(status, body)` otherwise.
    pub async fn json_request<B: Serialize>(
        &self,
        back_domain: &str,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<Value, AppError> {
        let (internal_url, token) = self.target(back_domain).await?;
        let url = format!("{}{}", internal_url.trim_end_matches('/'), path);
        let mut req = self.http.request(method, &url).bearer_auth(token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("backend '{back_domain}': {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
        } else {
            Err(AppError::BackendError(status.as_u16(), text))
        }
    }

    pub async fn get_json(&self, back_domain: &str, path: &str) -> Result<Value, AppError> {
        self.json_request::<()>(back_domain, Method::GET, path, None)
            .await
    }

    /// Pass-through proxy (feature 23 §5.3): forwards `method`+`path`(+JSON body) with the delegation
    /// bearer and returns `(status, body)` **without** turning a backend 4xx/5xx into an error, so the
    /// dashboard sees the real backend response.
    pub async fn proxy_json(
        &self,
        back_domain: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value), AppError> {
        let (internal_url, token) = self.target(back_domain).await?;
        let url = format!("{}{}", internal_url.trim_end_matches('/'), path);
        let mut req = self.http.request(method, &url).bearer_auth(token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("backend '{back_domain}': {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        Ok((
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        ))
    }

    /// Provision a user on the chosen backend (feature 23 §6.4): the backend accepts every
    /// resolver-forwarded signup; the resolver supplies `invited_by`.
    pub async fn register_user(
        &self,
        back_domain: &str,
        payload: &Value,
    ) -> Result<Value, AppError> {
        self.json_request(
            back_domain,
            Method::POST,
            "/api/resolver/users",
            Some(payload),
        )
            .await
    }
}
