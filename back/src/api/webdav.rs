//! WebDAV protocol adapter (06_webdav.md §4) — a hand-rolled handler over [`services::vfs`].
//!
//! Mounted at `/webdav/{slug}/...` outside `/api` (clients can't send a User JWT). HTTP Basic
//! auth resolves the per-hierarchy token (`services::webdav`). The fixed property set keeps the
//! PROPFIND/PROPPATCH XML small enough to build directly. Locking is advisory/fake (class 2 for
//! Finder): LOCK returns a token, nothing is enforced.

use crate::infra::error::AppError;
use crate::services::vfs::{ReadTarget, Vfs, VfsEntry};
use crate::services::webdav;
use crate::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use base64::Engine as _;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tracing::{trace, warn};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        // The PUT body is streamed to a temp file and bounded by `WEBDAV_MAX_UPLOAD_BYTES`
        // inline (§7); disable axum's default body limit so it does not cap streaming.
        .route("/webdav/{*rest}", any(handler))
        .layer(DefaultBodyLimit::disable())
}

async fn handler(State(state): State<AppState>, req: Request<Body>) -> Response {
    match dispatch(state, req).await {
        Ok(resp) => resp,
        Err(AppError::Unauthorized(_)) => unauthorized(),
        Err(e) => e.into_response(),
    }
}

// `method` and `path` are already carried by the ambient `http_request` span (main.rs's
// TraceLayer), so this span only adds what isn't known until Basic-auth resolves: the webdav
// user and target hierarchy.
#[tracing::instrument(skip_all, fields(token_type = "webdav", user, hierarchy))]
async fn dispatch(state: AppState, req: Request<Body>) -> Result<Response, AppError> {
    let method = req.method().clone();
    let uri_path = req.uri().path().to_string();
    let headers = req.headers().clone();

    let (slug, segments) = parse_mount_path(&uri_path)?;

    // OPTIONS is allowed pre-auth (clients probe before sending credentials).
    if method == Method::OPTIONS {
        return Ok(options_response());
    }

    let (username, token) = basic_auth(&headers)?;
    let session = webdav::authenticate(&state, &username, &token, &slug).await?;
    let span = tracing::Span::current();
    span.record("user", username.as_str());
    span.record("hierarchy", tracing::field::display(session.hierarchy_id));

    let vfs = Vfs::load(
        &state,
        session.user_id,
        session.hierarchy_id,
        session.use_redirect,
    )
    .await?;

    // macOS AppleDouble (`._*`) and other OS sidecar/junk files are not in the tag-derived tree.
    // They are stored as transient Redis sidecars so they round-trip in listings, but never get
    // ingested as pictures (06_webdav.md §11).
    if is_ignored(&segments) {
        return ignored(&state, &vfs, &method, &slug, &segments, &headers, req).await;
    }

    // Atomic-save ("safe-save") scratch paths bypass the tag tree: their bytes are staged until a
    // terminal rename promotes them to a real picture (08_webdav_issues.md §1).
    if is_atomic_staging(&segments) {
        return staging(&state, &vfs, &method, &slug, &segments, &headers, req).await;
    }
    // A real picture MOVEd/COPYd into a scratch path (the "move the original out of the way" step)
    // records a backup reference and mutates nothing (§1.5).
    if matches!(method.as_str(), "MOVE" | "COPY") {
        let dest = destination_segments(&headers)?;
        if is_atomic_staging(&dest) {
            vfs.stage_backup_ref(&segments, &dest).await?;
            return Ok(empty(StatusCode::CREATED));
        }
    }

    match method.as_str() {
        "PROPFIND" => propfind(&vfs, &slug, &segments, depth_header(&headers)).await,
        "GET" => read(&vfs, &segments, true).await,
        "HEAD" => read(&vfs, &segments, false).await,
        "PUT" => put(&state, &vfs, &segments, &headers, req).await,
        "DELETE" => {
            vfs.delete(&segments).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "MKCOL" => {
            vfs.mkcol(&segments).await?;
            Ok(empty(StatusCode::CREATED))
        }
        "MOVE" => {
            let dest = destination_segments(&headers)?;
            trace!(dest = %dest.join("/"), "webdav: destination");
            vfs.move_(&segments, &dest).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "COPY" => {
            let dest = destination_segments(&headers)?;
            trace!(dest = %dest.join("/"), "webdav: destination");
            vfs.copy(&segments, &dest).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "PROPPATCH" => Ok(proppatch_response(&slug, &segments)),
        "LOCK" => Ok(lock_response()),
        "UNLOCK" => Ok(empty(StatusCode::NO_CONTENT)),
        other => {
            // Not centrally logged: this returns a plain Response, not an AppError, so the
            // generic 4xx warn in `AppError::into_response` never sees it.
            warn!("webdav: unsupported method {other}");
            Ok(empty(StatusCode::METHOD_NOT_ALLOWED))
        }
    }
}

// ── Reads ───────────────────────────────────────────────────────────────────────

async fn read(vfs: &Vfs<'_>, segments: &[String], with_body: bool) -> Result<Response, AppError> {
    // A directory GET isn't meaningful (PROPFIND lists); reject so clients fall back.
    if segments.is_empty() || vfs.stat(segments).await.map(|e| e.is_dir).unwrap_or(false) {
        return Ok(empty(StatusCode::METHOD_NOT_ALLOWED));
    }
    match vfs.read_file(segments).await? {
        ReadTarget::Redirect(url) => Ok(Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, url)
            .body(Body::empty())
            .unwrap()),
        ReadTarget::Bytes { data, mime } => {
            let ct = mime.unwrap_or_else(|| "application/octet-stream".to_string());
            let len = data.len();
            let body = if with_body {
                Body::from(data)
            } else {
                Body::empty()
            };
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct)
                .header(header::CONTENT_LENGTH, len)
                .body(body)
                .unwrap())
        }
    }
}

// ── Writes ──────────────────────────────────────────────────────────────────────

async fn put(
    state: &AppState,
    vfs: &Vfs<'_>,
    segments: &[String],
    headers: &HeaderMap,
    req: Request<Body>,
) -> Result<Response, AppError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Stream the body to a temp file (never buffered in memory), then hash it with the common
    // crate's chunked hasher — we need the SHA-256 before deciding whether to upload to S3 or
    // just retag an existing picture (06_webdav.md §7–8).
    let (tmp, hash, size) =
        stream_to_temp(req.into_body(), state.config.webdav_max_upload_bytes).await?;

    let created = vfs
        .put_file(
            segments,
            tmp.path(),
            &hash,
            size as i64,
            content_type.as_deref(),
        )
        .await?;
    // `tmp` is dropped here, removing the temp file.
    Ok(empty(if created {
        StatusCode::CREATED
    } else {
        StatusCode::NO_CONTENT
    }))
}

/// Stream a request body to a temporary file, enforcing `max_bytes`, and return the file handle
/// (keep it alive to retain the file), its SHA-256 hex digest, and its byte length. Hashing reads
/// the finished file in `spawn_blocking` so the async runtime is never blocked (06_webdav.md §7).
#[tracing::instrument(skip_all, fields(bytes))]
async fn stream_to_temp(
    body: Body,
    max_bytes: u64,
) -> Result<(tempfile::NamedTempFile, String, u64), AppError> {
    let tmp = tokio::task::spawn_blocking(tempfile::NamedTempFile::new)
        .await
        .map_err(|e| AppError::InternalServerError(format!("temp file task: {e}")))?
        .map_err(|e| AppError::InternalServerError(format!("create temp file: {e}")))?;
    let path = tmp.path().to_path_buf();

    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| AppError::InternalServerError(format!("open temp file: {e}")))?;
    let mut stream = body.into_data_stream();
    let mut size: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::BadRequest(format!("failed to read body: {e}")))?;
        size += chunk.len() as u64;
        if size > max_bytes {
            return Err(AppError::PayloadTooLarge(format!(
                "upload exceeds WEBDAV_MAX_UPLOAD_BYTES ({max_bytes} bytes)"
            )));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::InternalServerError(format!("write temp file: {e}")))?;
    }
    file.flush()
        .await
        .map_err(|e| AppError::InternalServerError(format!("flush temp file: {e}")))?;
    drop(file);

    let hash = tokio::task::spawn_blocking(move || archypix_common::hash::hash_file(&path))
        .await
        .map_err(|e| AppError::InternalServerError(format!("hash task: {e}")))?
        .ok_or_else(|| AppError::InternalServerError("failed to hash uploaded file".into()))?;

    tracing::Span::current().record("bytes", size);
    Ok((tmp, hash, size))
}

// ── PROPFIND ──────────────────────────────────────────────────────────────────────

async fn propfind(
    vfs: &Vfs<'_>,
    slug: &str,
    segments: &[String],
    depth: Depth,
) -> Result<Response, AppError> {
    if matches!(depth, Depth::Infinity) {
        // Resolving the whole tree is expensive; RFC 4918 permits refusing infinity.
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(Body::from(
                "<error>propfind-finite-depth: Depth: infinity is not supported</error>",
            ))
            .unwrap());
    }

    let here = vfs.stat(segments).await?;
    // RFC 4331 quota properties are advertised on collections (feature 22 §8.2). Resolve them once.
    let quota = vfs.quota_props().await.ok();
    let mut responses = String::new();
    responses.push_str(&response_xml(slug, segments, &here, quota));

    if matches!(depth, Depth::One) && here.is_dir {
        let entries = vfs.list_dir(segments).await?;
        for e in &entries {
            let mut child = segments.to_vec();
            child.push(e.name.clone());
            responses.push_str(&response_xml(slug, &child, e, quota));
        }
    }

    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">{responses}</D:multistatus>"#
    );
    Ok(Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .body(Body::from(body))
        .unwrap())
}

fn response_xml(
    slug: &str,
    segments: &[String],
    entry: &VfsEntry,
    quota: Option<(i64, Option<i64>)>,
) -> String {
    let href = href_for(slug, segments, entry.is_dir);
    let modified = entry
        .modified
        .and_utc()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let mut props = String::new();
    props.push_str(&format!(
        "<D:displayname>{}</D:displayname>",
        xml_escape(&entry.name)
    ));
    props.push_str(&format!(
        "<D:getlastmodified>{modified}</D:getlastmodified>"
    ));
    if entry.is_dir {
        props.push_str("<D:resourcetype><D:collection/></D:resourcetype>");
        // RFC 4331 capacity bar. `quota-used-bytes` is the billed total; `quota-available-bytes` is
        // omitted when the quota is unlimited.
        if let Some((used, available)) = quota {
            props.push_str(&format!("<D:quota-used-bytes>{used}</D:quota-used-bytes>"));
            if let Some(avail) = available {
                props.push_str(&format!(
                    "<D:quota-available-bytes>{avail}</D:quota-available-bytes>"
                ));
            }
        }
    } else {
        props.push_str("<D:resourcetype/>");
        props.push_str(&format!(
            "<D:getcontentlength>{}</D:getcontentlength>",
            entry.size
        ));
        if let Some(ct) = &entry.mime_type {
            props.push_str(&format!(
                "<D:getcontenttype>{}</D:getcontenttype>",
                xml_escape(ct)
            ));
        }
        if let Some(etag) = &entry.etag {
            props.push_str(&format!("<D:getetag>\"{}\"</D:getetag>", xml_escape(etag)));
        }
    }
    format!(
        r#"<D:response><D:href>{href}</D:href><D:propstat><D:prop>{props}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>"#
    )
}

fn proppatch_response(slug: &str, segments: &[String]) -> Response {
    // No-op: accept any property set so clients (Finder) that PROPPATCH mtime succeed.
    let href = href_for(slug, segments, false);
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>{href}</D:href><D:propstat><D:prop/><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
    );
    Response::builder()
        .status(StatusCode::MULTI_STATUS)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

fn lock_response() -> Response {
    // Fake advisory lock (class 2) so Finder proceeds; nothing is actually enforced.
    let token = format!("opaquelocktoken:{}", Uuid::new_v4());
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<D:prop xmlns:D="DAV:"><D:lockdiscovery><D:activelock>
<D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope>
<D:depth>0</D:depth><D:timeout>Second-3600</D:timeout>
<D:locktoken><D:href>{token}</D:href></D:locktoken>
</D:activelock></D:lockdiscovery></D:prop>"#
    );
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .header("Lock-Token", format!("<{token}>"))
        .body(Body::from(body))
        .unwrap()
}

fn options_response() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("DAV", "1, 2")
        .header("MS-Author-Via", "DAV")
        .header(
            header::ALLOW,
            "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, MKCOL, COPY, MOVE, LOCK, UNLOCK",
        )
        .header(header::CONTENT_LENGTH, 0)
        .body(Body::empty())
        .unwrap()
}

// ── Atomic-save staging (08_webdav_issues.md §1) ────────────────────────────────────

/// Whether any path segment is an atomic-save scratch artifact — a temp directory or file a
/// client writes then renames over the target (08_webdav_issues.md §1.4). Checked across all
/// segments because macOS nests the real file inside a `.sb-…` temp directory.
fn is_atomic_staging(segments: &[String]) -> bool {
    segments.iter().any(|s| is_atomic_staging_name(s))
}

fn is_atomic_staging_name(name: &str) -> bool {
    if is_macos_safe_save(name) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    // Windows ReplaceFile / editors: temp file then rename over the target.
    lower.ends_with(".tmp")
        || name.ends_with('~')
        // Browser / rsync / GVFS partial-download temporaries.
        || lower.ends_with(".part")
        || lower.ends_with(".partial")
        || lower.ends_with(".crdownload")
        || lower.ends_with(".download")
        || name.starts_with(".goutputstream-")
}

/// macOS `NSDocument` safe-save scratch name: `<base>.sb-<8 hex>-<6 alnum>` (dir or file).
fn is_macos_safe_save(name: &str) -> bool {
    let Some(idx) = name.rfind(".sb-") else {
        return false;
    };
    let Some((hex, rand)) = name[idx + 4..].split_once('-') else {
        return false;
    };
    hex.len() == 8
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
        && rand.len() == 6
        && rand.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Handle a request whose target is an atomic-save scratch path (08_webdav_issues.md §1). Scratch
/// bytes live in the staging bucket + a Redis marker; a terminal MOVE promotes them to a picture.
async fn staging(
    state: &AppState,
    vfs: &Vfs<'_>,
    method: &Method,
    slug: &str,
    segments: &[String],
    headers: &HeaderMap,
    req: Request<Body>,
) -> Result<Response, AppError> {
    match method.as_str() {
        "PUT" => {
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let (tmp, hash, size) =
                stream_to_temp(req.into_body(), state.config.webdav_max_upload_bytes).await?;
            // An empty placeholder PUT (Finder/Preview issue one first) — accept, stage nothing.
            if size == 0 {
                return Ok(empty(StatusCode::CREATED));
            }
            vfs.put_staging(
                segments,
                tmp.path(),
                &hash,
                size as i64,
                content_type.as_deref(),
            )
            .await?;
            Ok(empty(StatusCode::CREATED))
        }
        "GET" | "HEAD" => match vfs.read_staging(segments).await? {
            Some(ReadTarget::Redirect(url)) => Ok(Response::builder()
                .status(StatusCode::FOUND)
                .header(header::LOCATION, url)
                .body(Body::empty())
                .unwrap()),
            Some(ReadTarget::Bytes { data, mime }) => {
                let ct = mime.unwrap_or_else(|| "application/octet-stream".to_string());
                let len = data.len();
                let body = if method == Method::GET {
                    Body::from(data)
                } else {
                    Body::empty()
                };
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, ct)
                    .header(header::CONTENT_LENGTH, len)
                    .body(body)
                    .unwrap())
            }
            None => Ok(empty(StatusCode::NOT_FOUND)),
        },
        // PROPFIND/stat/list surface staged entries directly (see `services::vfs`). A missing
        // scratch path (pre-PUT probe) returns 404 without a spurious warn.
        "PROPFIND" => match propfind(vfs, slug, segments, depth_header(headers)).await {
            Err(AppError::NotFound) => Ok(empty(StatusCode::NOT_FOUND)),
            other => other,
        },
        "MKCOL" => {
            vfs.add_staging_dir(segments).await?;
            Ok(empty(StatusCode::CREATED))
        }
        "DELETE" => {
            vfs.delete_staging(segments).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "MOVE" => {
            let dest = destination_segments(headers)?;
            trace!(dest = %dest.join("/"), "webdav staging: move");
            if is_atomic_staging(&dest) {
                // scratch → scratch: just relocate the marker.
                vfs.move_staging(segments, &dest).await?;
                Ok(empty(StatusCode::NO_CONTENT))
            } else {
                // scratch → real: the terminal rename. Promote the staged bytes.
                let created = vfs.promote_staging(segments, &dest, true).await?;
                Ok(empty(if created {
                    StatusCode::CREATED
                } else {
                    StatusCode::NO_CONTENT
                }))
            }
        }
        "COPY" => {
            let dest = destination_segments(headers)?;
            if is_atomic_staging(&dest) {
                Ok(empty(StatusCode::CREATED))
            } else {
                let created = vfs.promote_staging(segments, &dest, false).await?;
                Ok(empty(if created {
                    StatusCode::CREATED
                } else {
                    StatusCode::NO_CONTENT
                }))
            }
        }
        "LOCK" => Ok(lock_response()),
        "UNLOCK" => Ok(empty(StatusCode::NO_CONTENT)),
        "PROPPATCH" => Ok(proppatch_response(slug, segments)),
        other => {
            warn!("webdav staging: unsupported method {other}");
            Ok(empty(StatusCode::METHOD_NOT_ALLOWED))
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────────

/// Whether the target is an OS sidecar/junk file that should never become a picture (§11).
fn is_ignored(segments: &[String]) -> bool {
    segments.last().is_some_and(|n| is_ignored_name(n))
}

fn is_ignored_name(name: &str) -> bool {
    // AppleDouble companions (`._Foo`, and `._.` for the directory itself).
    name.starts_with("._")
        || matches!(
            name,
            ".DS_Store"
                | ".localized"
                | ".hidden"
                | "Thumbs.db"
                | "desktop.ini"
                | ".metadata_never_index"
                | ".TemporaryItems"
                | ".Trashes"
                | ".fseventsd"
                | ".apdisk"
                | ".com.apple.timemachine.donotpresent"
        )
}

/// Handle a request for an OS-junk sidecar path (06_webdav.md §11): store/serve/list it from the
/// transient Redis sidecar cache so clients see it round-trip, but never ingest it as a picture.
async fn ignored(
    state: &AppState,
    vfs: &Vfs<'_>,
    method: &Method,
    slug: &str,
    segments: &[String],
    headers: &HeaderMap,
    req: Request<Body>,
) -> Result<Response, AppError> {
    match method.as_str() {
        "PUT" => {
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            // Sidecars are tiny; buffer with a small cap (oversized bodies are accepted but not stored).
            let bytes = axum::body::to_bytes(
                req.into_body(),
                state.config.webdav_max_upload_bytes as usize,
            )
            .await
            .map_err(|e| AppError::BadRequest(format!("failed to read body: {e}")))?;
            vfs.put_sidecar(segments, &bytes, content_type.as_deref())
                .await?;
            Ok(empty(StatusCode::CREATED))
        }
        "GET" | "HEAD" => match vfs.read_sidecar(segments).await? {
            Some((data, mime)) => {
                let ct = mime.unwrap_or_else(|| "application/octet-stream".to_string());
                let len = data.len();
                let body = if method == Method::GET {
                    Body::from(data)
                } else {
                    Body::empty()
                };
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, ct)
                    .header(header::CONTENT_LENGTH, len)
                    .body(body)
                    .unwrap())
            }
            None => Ok(empty(StatusCode::NOT_FOUND)),
        },
        // PROPFIND uses the normal resolver — `stat` surfaces the sidecar (Depth 0 is all that's
        // meaningful on a file). A missing sidecar (pre-PUT probe) returns 404 as an Ok response
        // so it never reaches `AppError::into_response()` and does not produce a spurious warn.
        "PROPFIND" => match propfind(vfs, slug, segments, Depth::Zero).await {
            Err(AppError::NotFound) => Ok(empty(StatusCode::NOT_FOUND)),
            other => other,
        },
        "DELETE" => {
            vfs.delete_sidecar(segments).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        // Structure-only operations stay benign so the client doesn't hang or error.
        "MKCOL" => Ok(empty(StatusCode::CREATED)),
        "MOVE" | "COPY" | "UNLOCK" => Ok(empty(StatusCode::NO_CONTENT)),
        "LOCK" => Ok(lock_response()),
        "PROPPATCH" => Ok(proppatch_response(slug, segments)),
        _ => Ok(empty(StatusCode::NOT_FOUND)),
    }
}

enum Depth {
    Zero,
    One,
    Infinity,
}

fn depth_header(headers: &HeaderMap) -> Depth {
    match headers.get("Depth").and_then(|v| v.to_str().ok()) {
        Some("0") => Depth::Zero,
        Some("infinity") => Depth::Infinity,
        // Default to 1 (most clients send it explicitly; never default to infinity).
        _ => Depth::One,
    }
}

fn empty(status: StatusCode) -> Response {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap()
}

fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(header::WWW_AUTHENTICATE, "Basic realm=\"Archypix WebDAV\"")
        .body(Body::empty())
        .unwrap()
}

fn basic_auth(headers: &HeaderMap) -> Result<(String, String), AppError> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("missing credentials".into()))?;
    let b64 = raw
        .strip_prefix("Basic ")
        .ok_or_else(|| AppError::Unauthorized("expected Basic auth".into()))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| AppError::Unauthorized("invalid base64".into()))?;
    let s = String::from_utf8(decoded)
        .map_err(|_| AppError::Unauthorized("invalid credentials".into()))?;
    let (user, pass) = s
        .split_once(':')
        .ok_or_else(|| AppError::Unauthorized("malformed credentials".into()))?;
    Ok((user.to_string(), pass.to_string()))
}

/// Strip the `/webdav/` prefix and split into (slug, decoded path segments).
fn parse_mount_path(path: &str) -> Result<(String, Vec<String>), AppError> {
    let mut parts = path.split('/').filter(|s| !s.is_empty());
    match parts.next() {
        Some("webdav") => {}
        _ => return Err(AppError::NotFound),
    }
    let slug = parts.next().map(percent_decode).ok_or(AppError::NotFound)?;
    let segments: Vec<String> = parts.map(percent_decode).collect();
    Ok((slug, segments))
}

/// Parse the `Destination` header (MOVE/COPY) into path segments under the mount.
fn destination_segments(headers: &HeaderMap) -> Result<Vec<String>, AppError> {
    let raw = headers
        .get("Destination")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("missing Destination header".into()))?;
    // Tolerate both absolute URLs and absolute paths.
    let idx = raw
        .find("/webdav/")
        .ok_or_else(|| AppError::BadRequest("Destination not under /webdav".into()))?;
    let (_slug, segments) = parse_mount_path(&raw[idx..])?;
    Ok(segments)
}

fn href_for(slug: &str, segments: &[String], is_dir: bool) -> String {
    let mut href = format!("/webdav/{}", encode_segment(slug));
    for seg in segments {
        href.push('/');
        href.push_str(&encode_segment(seg));
    }
    if is_dir {
        href.push('/');
    }
    href
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Percent-decode a single path segment (handles `%XX`; leaves other bytes as-is).
fn percent_decode(seg: &str) -> String {
    let bytes = seg.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Percent-encode a path segment, leaving RFC 3986 unreserved characters intact.
fn encode_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for &b in seg.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mount_path_splits_slug_and_segments() {
        let (slug, seg) = parse_mount_path("/webdav/my-photos/Travel/IMG%20001.jpg").unwrap();
        assert_eq!(slug, "my-photos");
        assert_eq!(seg, vec!["Travel".to_string(), "IMG 001.jpg".to_string()]);
    }

    #[test]
    fn parse_mount_path_root_of_mount() {
        let (slug, seg) = parse_mount_path("/webdav/photos").unwrap();
        assert_eq!(slug, "photos");
        assert!(seg.is_empty());
    }

    #[test]
    fn href_encodes_and_trails_dirs() {
        assert_eq!(
            href_for("photos", &["A B".to_string()], true),
            "/webdav/photos/A%20B/"
        );
        assert_eq!(
            href_for("photos", &["x.jpg".to_string()], false),
            "/webdav/photos/x.jpg"
        );
    }

    #[test]
    fn xml_escape_basic() {
        assert_eq!(xml_escape("a&b<c>"), "a&amp;b&lt;c&gt;");
    }

    #[test]
    fn round_trip_percent() {
        assert_eq!(percent_decode(&encode_segment("a b/c")), "a b/c");
    }

    #[test]
    fn ignores_appledouble_and_os_junk() {
        for n in [
            "._.",
            "._Minecraft",
            "._To-ArveniaNorth_left.png",
            ".DS_Store",
            ".localized",
            "Thumbs.db",
            "desktop.ini",
        ] {
            assert!(is_ignored_name(n), "{n} should be ignored");
        }
    }

    #[test]
    fn does_not_ignore_real_files() {
        for n in [
            "Minecraft",
            "Paradise_Point.png",
            "photo.jpg",
            "_underscore.png",
        ] {
            assert!(!is_ignored_name(n), "{n} should not be ignored");
        }
        assert!(is_ignored(&["dir".into(), "._x.png".into()]));
        assert!(!is_ignored(&["dir".into(), "x.png".into()]));
    }

    #[test]
    fn recognizes_atomic_save_scratch_names() {
        for n in [
            "phare.jpg.sb-93035015-3rqb93", // macOS temp dir
            "phare.jpg.sb-93035015-oDc68c",
            "report.docx.tmp", // Windows ReplaceFile
            "notes.txt~",      // editor backup-then-rename
            "movie.mp4.part",  // partial download
            "movie.mp4.partial",
            "song.crdownload",
            ".goutputstream-AB12CD", // GVFS
        ] {
            assert!(is_atomic_staging_name(n), "{n} should be atomic-staging");
        }
    }

    #[test]
    fn does_not_treat_real_files_as_scratch() {
        for n in [
            "phare.jpg",
            "phare.sb.jpg",            // not the `.sb-<hex>-<alnum>` shape
            "phare.jpg.sb-xyz-123",    // hex/len wrong
            "phare.jpg.sb-93035015-3", // rand too short
            "temperature.png",         // not a `.tmp` suffix
        ] {
            assert!(
                !is_atomic_staging_name(n),
                "{n} should not be atomic-staging"
            );
        }
        // Detection spans ancestors: the real file lives inside a `.sb-…` temp directory.
        assert!(is_atomic_staging(&[
            "Photos".into(),
            "phare.jpg.sb-93035015-3rqb93".into(),
            "phare.jpg".into(),
        ]));
        assert!(!is_atomic_staging(&["Photos".into(), "phare.jpg".into()]));
    }
}
