use crate::AppState;
use crate::database::{
    count_users_per_backend, get_backend_url, list_backends, upsert_backend, upsert_mapping,
    username_exists,
};
use crate::error::AppError;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

// ── WebFinger ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WebFingerQuery {
    resource: String,
}
#[derive(Debug, Serialize)]
pub struct WebFingerResponse {
    subject: String,
    links: Vec<WebFingerLink>,
}
#[derive(Debug, Serialize)]
pub struct WebFingerLink {
    rel: String,
    href: String,
}

pub async fn webfinger_handler(
    Query(query): Query<WebFingerQuery>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let username = parse_acct_resource(&query.resource, &state)?;

    if let Some(backend_url) = state.cache.get(&username).await {
        debug!(user = %username, token_type = "-", source = "cache", "webfinger");
        return Ok(jrd_response(build_webfinger_response(
            &username,
            &backend_url,
            &state.global_domain,
        )));
    }

    let backend_url = get_backend_url(&state.db, &username).await?;

    if let Some(backend_url) = backend_url {
        debug!(user = %username, token_type = "-", source = "db", "webfinger");
        state
            .cache
            .insert(username.clone(), backend_url.clone())
            .await;
        Ok(jrd_response(build_webfinger_response(
            &username,
            &backend_url,
            &state.global_domain,
        )))
    } else {
        warn!(user = %username, token_type = "-", "webfinger: username not found");
        Err(AppError::NotFound)
    }
}

/// Serialize a WebFinger response with the RFC 7033-mandated Content-Type.
fn jrd_response(body: WebFingerResponse) -> Response {
    let json = serde_json::to_string(&body).expect("WebFingerResponse is always serializable");
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/jrd+json"),
        )],
        json,
    )
        .into_response()
}

// ── Update mapping ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    username: String,
    back_domain: String,
}
#[derive(Debug, Serialize)]
pub struct UpdateResponse {
    success: bool,
    message: String,
}

pub async fn update_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateRequest>,
) -> Result<Json<UpdateResponse>, AppError> {
    let token = extract_bearer_token(&headers)?;
    verify_resolver_jwt(token, &state)?;

    if payload.username.is_empty() || payload.back_domain.is_empty() {
        return Err(AppError::BadRequest(
            "username and back_domain are required".to_string(),
        ));
    }

    upsert_mapping(&state.db, &payload.username, &payload.back_domain).await?;
    // Invalidate the cache entry so the next WebFinger query rebuilds it from the DB.
    state.cache.invalidate(&payload.username).await;

    debug!(
        user = %payload.username,
        token_type = "resolver",
        back_domain = %payload.back_domain,
        "update_mapping"
    );

    Ok(Json(UpdateResponse {
        success: true,
        message: format!("Mapping updated for user {}", payload.username),
    }))
}

// ── Backend self-registration ─────────────────────────────────────────────────

/// Payload sent by a backend at startup to register itself with the resolver.
#[derive(Debug, Deserialize)]
pub struct RegisterBackendRequest {
    /// Public-facing domain (and optional port) of this backend, e.g. `backend1.example.com`
    /// or `localhost:8001`. Used as JWT audience and to derive the public URL.
    pub back_domain: String,
    /// Whether this backend is served over HTTPS. Combined with `back_domain` to produce the
    /// public URL that is stored in WebFinger responses.
    pub use_https: bool,
    /// Internal URL the resolver should use for API calls, e.g. `http://backend1:8000`.
    pub internal_url: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterBackendResponse {
    success: bool,
    message: String,
}

pub async fn register_backend_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RegisterBackendRequest>,
) -> Result<Json<RegisterBackendResponse>, AppError> {
    let token = extract_bearer_token(&headers)?;
    verify_resolver_jwt(token, &state)?;

    if payload.back_domain.is_empty() || payload.internal_url.is_empty() {
        return Err(AppError::BadRequest(
            "back_domain and internal_url are required".to_string(),
        ));
    }

    upsert_backend(
        &state.db,
        &payload.back_domain,
        payload.use_https,
        &payload.internal_url,
    )
    .await?;

    info!(
        back_domain = %payload.back_domain,
        use_https = payload.use_https,
        internal_url = %payload.internal_url,
        "Backend registered"
    );

    Ok(Json(RegisterBackendResponse {
        success: true,
        message: format!("Backend {} registered", payload.back_domain),
    }))
}

pub async fn list_backends_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = extract_bearer_token(&headers)?;
    verify_resolver_jwt(token, &state)?;

    let backends = list_backends(&state.db).await?;
    Ok(Json(serde_json::json!({ "backends": backends })))
}

// ── User registration (forwarded to a backend) ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    username: String,
    display_name: String,
    email: String,
    password: String,
}
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    username: String,
    backend_url: String,
    message: String,
}

pub async fn register_handler(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<RegisterResponse>, AppError> {
    debug!(user = %payload.username, token_type = "-", "register");
    if payload.username.is_empty() || payload.email.is_empty() {
        return Err(AppError::BadRequest(
            "username and email are required".to_string(),
        ));
    }

    if username_exists(&state.db, &payload.username).await? {
        return Err(AppError::BadRequest(format!(
            "Username '{}' is already taken",
            payload.username
        )));
    }

    let backend_counts = count_users_per_backend(&state.db).await?;
    if backend_counts.is_empty() {
        return Err(AppError::ServiceUnavailable(
            "No backend nodes registered".to_string(),
        ));
    }

    let (chosen_back_domain, chosen_use_https, chosen_internal_url, _) = &backend_counts[0];
    let scheme = if *chosen_use_https { "https" } else { "http" };
    let chosen_backend_url = format!("{}://{}", scheme, chosen_back_domain);

    let resolver_jwt = generate_resolver_jwt(&state, chosen_back_domain)?;

    let backend_register_url = format!("{}/api/resolver/users", chosen_internal_url);
    let body = serde_json::json!({
        "username": payload.username,
        "display_name": payload.display_name,
        "email": payload.email,
        "password": payload.password,
    });

    let response = state
        .reqwest_client
        .post(&backend_register_url)
        .header("Authorization", format!("Bearer {}", resolver_jwt))
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            warn!(
                "Failed to contact backend {} (internal: {}): {}",
                chosen_back_domain, chosen_internal_url, e
            );
            AppError::ServiceUnavailable(format!("Failed to contact backend: {e}"))
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "unknown error".to_string());
        warn!(
            "Backend {} returned error {}: {}",
            chosen_back_domain, status, error_body
        );
        return Err(AppError::BackendError(status.as_u16(), error_body));
    }

    upsert_mapping(&state.db, &payload.username, chosen_back_domain).await?;

    debug!(
        user = %payload.username,
        token_type = "-",
        back_domain = %chosen_back_domain,
        "register: user registered"
    );

    Ok(Json(RegisterResponse {
        username: payload.username,
        backend_url: chosen_backend_url,
        message: "User registered successfully".to_string(),
    }))
}

// ── Health ────────────────────────────────────────────────────────────────────

pub async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "archypix-resolver"
    }))
}

// ── JWT helpers ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ResolverJwtClaims {
    #[allow(dead_code)]
    sub: String,
    #[allow(dead_code)]
    is_admin: bool,
    #[allow(dead_code)]
    instance: String,
    token_type: String,
    #[allow(dead_code)]
    aud: String,
    #[allow(dead_code)]
    iss: String,
    #[allow(dead_code)]
    exp: i64,
    #[allow(dead_code)]
    iat: i64,
    #[allow(dead_code)]
    jti: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResolverJwtClaimsEncode {
    sub: String,
    is_admin: bool,
    instance: String,
    token_type: String,
    aud: String,
    iss: String,
    exp: i64,
    iat: i64,
    jti: String,
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, AppError> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| {
            AppError::Unauthorized("Missing or invalid Authorization header".to_string())
        })
}

fn verify_resolver_jwt(token: &str, state: &AppState) -> Result<ResolverJwtClaims, AppError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_audience(&[state.global_domain.clone()]);
    let data = decode::<ResolverJwtClaims>(
        token,
        &DecodingKey::from_secret(state.resolver_jwt_secret.as_bytes()),
        &validation,
    )
    .map_err(|_| AppError::Unauthorized("Invalid resolver JWT".to_string()))?;

    if data.claims.token_type != "resolver" {
        return Err(AppError::Unauthorized(
            "Invalid token type: expected resolver".to_string(),
        ));
    }

    Ok(data.claims)
}

/// Generate a short-lived JWT the resolver sends to a backend when forwarding registration.
/// Audience is the backend's `back_domain` so only that backend can accept the token.
fn generate_resolver_jwt(state: &AppState, back_domain: &str) -> Result<String, AppError> {
    let now = Utc::now().timestamp();
    let claims = ResolverJwtClaimsEncode {
        sub: "resolver".to_string(),
        is_admin: false,
        instance: state.global_domain.clone(),
        token_type: "resolver".to_string(),
        aud: back_domain.to_string(),
        iss: "resolver".to_string(),
        exp: now + 300,
        iat: now,
        jti: Uuid::new_v4().to_string(),
    };

    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(state.resolver_jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::InternalServerError(e.to_string()))
}

// ── Resource parsing / response building ──────────────────────────────────────

fn parse_acct_resource(resource: &str, state: &AppState) -> Result<String, AppError> {
    let rest = resource.strip_prefix("archypix:@").ok_or_else(|| {
        AppError::BadRequest("Invalid resource format. Expected archypix:@user:domain".to_string())
    })?;

    // splitn(2) keeps a domain:port like `localhost:8001` intact as the second part.
    let mut iter = rest.splitn(2, ':');
    let user = iter.next().ok_or_else(|| {
        AppError::BadRequest("Invalid resource format. Expected archypix:@user:domain".to_string())
    })?;
    let domain = iter.next().ok_or_else(|| {
        AppError::BadRequest("Invalid resource format. Expected archypix:@user:domain".to_string())
    })?;

    if domain != state.global_domain {
        return Err(AppError::BadRequest(format!("Invalid domain: {}", domain)));
    }

    Ok(user.to_string())
}

fn build_webfinger_response(
    username: &str,
    backend_url: &str,
    global_domain: &str,
) -> WebFingerResponse {
    WebFingerResponse {
        subject: format!("archypix:@{}:{}", username, global_domain),
        links: vec![WebFingerLink {
            rel: "backend_url".to_string(),
            href: backend_url.to_string(),
        }],
    }
}
