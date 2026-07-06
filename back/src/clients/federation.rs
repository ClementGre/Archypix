mod handshake;
pub mod models;
mod shares;
mod webfinger;

use crate::infra::crypto::JwtService;
use crate::infra::observability;
use crate::infra::redis::Cache;
use crate::infra::settings::keys;
use archypix_common::settings::Settings;
use reqwest::Client as HttpClient;
use std::sync::Arc;

/// Outbound client for webfinger, federation auth, and protocol messages.
#[derive(Clone)]
pub struct FederationClient {
    pub(super) http: HttpClient,
    pub(super) settings: Arc<Settings>,
    pub(super) jwt: JwtService,
    pub(super) cache: Arc<dyn Cache>,
}

impl FederationClient {
    pub fn new(
        http: HttpClient,
        settings: Arc<Settings>,
        jwt: JwtService,
        cache: Arc<dyn Cache>,
    ) -> Self {
        Self {
            http,
            settings,
            jwt,
            cache,
        }
    }

    /// Build a `HeaderMap` with the current trace context injected, if `recipient_global_domain`
    /// is in the configured allowlist. Returns empty headers for non-allowlisted peers.
    pub(super) fn trace_headers_for(
        &self,
        recipient_global_domain: &str,
    ) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if self
            .settings
            .get(keys::TRACE_PROPAGATION_PEERS)
            .iter()
            .any(|p| p == recipient_global_domain)
        {
            observability::inject_into_headers(&mut headers);
        }
        headers
    }
}
