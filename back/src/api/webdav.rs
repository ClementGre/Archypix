//! WebDAV protocol adapter (06_webdav.md §4) — a hand-rolled handler over [`services::vfs`].
//!
//! Mounted at `/webdav/{slug}/...` outside `/api` (clients can't send a User JWT). HTTP Basic
//! auth resolves the per-hierarchy token (`services::webdav`). The fixed property set keeps the
//! PROPFIND/PROPPATCH XML small enough to build directly. Locking is advisory/fake (class 2 for
//! Finder): LOCK returns a token, nothing is enforced.

use crate::infra::error::AppError;
use crate::services::vfs::{ReadTarget, Vfs, VfsEntry};
use crate::services::webdav::{self, WebdavSession};
use crate::state::AppState;
use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use base64::Engine as _;
use tracing::debug;
use uuid::Uuid;

/// Upper bound on a single PUT body buffered in memory (guard; real photos are far smaller).
const MAX_UPLOAD_BYTES: usize = 5 * 1024 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/webdav/{*rest}", any(handler))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}

async fn handler(State(state): State<AppState>, req: Request<Body>) -> Response {
    match dispatch(state, req).await {
        Ok(resp) => resp,
        Err(AppError::Unauthorized(_)) => unauthorized(),
        Err(e) => e.into_response(),
    }
}

async fn dispatch(state: AppState, req: Request<Body>) -> Result<Response, AppError> {
    let method = req.method().clone();
    let uri_path = req.uri().path().to_string();
    let headers = req.headers().clone();

    let (slug, segments) = parse_mount_path(&uri_path)?;

    // OPTIONS is allowed pre-auth (clients probe before sending credentials).
    if method == Method::OPTIONS {
        debug!(user = "-", token_type = "-", %slug, "webdav OPTIONS");
        return Ok(options_response());
    }

    let (username, token) = basic_auth(&headers)?;
    let session = webdav::authenticate(&state, &username, &token, &slug).await?;

    // macOS AppleDouble (`._*`) and other OS sidecar/junk files are not in the tag-derived
    // tree. Short-circuit them so they neither 404-spam the logs nor get ingested as pictures
    // on PUT (06_webdav.md §11): quiet 404 on read, accept-and-discard on write.
    if is_ignored(&segments) {
        tracing::trace!(user = %username, path = %segments.join("/"), method = %method, "webdav ignored OS sidecar file");
        return Ok(ignored_response(&method, &slug, &segments));
    }

    let vfs = Vfs::load(
        &state,
        session.user_id,
        session.hierarchy_id,
        session.use_redirect,
    )
    .await?;

    // Common fields for the per-endpoint debug records (tracing policy: one debug per handler).
    let hierarchy = session.hierarchy_id;
    let path = segments.join("/");

    match method.as_str() {
        "PROPFIND" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav PROPFIND");
            propfind(&vfs, &slug, &segments, depth_header(&headers)).await
        }
        "GET" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav GET");
            read(&vfs, &segments, true).await
        }
        "HEAD" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav HEAD");
            read(&vfs, &segments, false).await
        }
        "PUT" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav PUT");
            put(&state, &session, &vfs, &segments, &headers, req).await
        }
        "DELETE" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav DELETE");
            vfs.delete(&segments).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "MKCOL" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav MKCOL");
            vfs.mkcol(&segments)?;
            Ok(empty(StatusCode::CREATED))
        }
        "MOVE" => {
            let dest = destination_segments(&headers)?;
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, dest = %dest.join("/"), "webdav MOVE");
            vfs.move_(&segments, &dest).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "COPY" => {
            let dest = destination_segments(&headers)?;
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, dest = %dest.join("/"), "webdav COPY");
            vfs.copy(&segments, &dest).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        "PROPPATCH" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav PROPPATCH");
            Ok(proppatch_response(&slug, &segments))
        }
        "LOCK" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav LOCK");
            Ok(lock_response())
        }
        "UNLOCK" => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, "webdav UNLOCK");
            Ok(empty(StatusCode::NO_CONTENT))
        }
        other => {
            debug!(user = %username, token_type = "webdav", %hierarchy, %path, method = %other, "webdav unsupported method");
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
    _session: &WebdavSession,
    vfs: &Vfs<'_>,
    segments: &[String],
    headers: &HeaderMap,
    req: Request<Body>,
) -> Result<Response, AppError> {
    let _ = state;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = axum::body::to_bytes(req.into_body(), MAX_UPLOAD_BYTES)
        .await
        .map_err(|e| AppError::BadRequest(format!("failed to read body: {e}")))?;
    let created = vfs
        .put_file(segments, bytes.to_vec(), content_type.as_deref())
        .await?;
    Ok(empty(if created {
        StatusCode::CREATED
    } else {
        StatusCode::NO_CONTENT
    }))
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
    let mut responses = String::new();
    responses.push_str(&response_xml(slug, segments, &here));

    if matches!(depth, Depth::One) && here.is_dir {
        let entries = vfs.list_dir(segments).await?;
        for e in &entries {
            let mut child = segments.to_vec();
            child.push(e.name.clone());
            responses.push_str(&response_xml(slug, &child, e));
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

fn response_xml(slug: &str, segments: &[String], entry: &VfsEntry) -> String {
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

/// Benign response for an ignore-listed path: quiet `404` on read so the client learns the
/// sidecar doesn't exist; accept-and-discard on write so the client doesn't hang or error.
fn ignored_response(method: &Method, slug: &str, segments: &[String]) -> Response {
    match method.as_str() {
        "PUT" | "MKCOL" => empty(StatusCode::CREATED),
        "DELETE" | "MOVE" | "COPY" | "UNLOCK" => empty(StatusCode::NO_CONTENT),
        "LOCK" => lock_response(),
        "PROPPATCH" => proppatch_response(slug, segments),
        // PROPFIND / GET / HEAD and anything else: the sidecar does not exist.
        _ => empty(StatusCode::NOT_FOUND),
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
}
