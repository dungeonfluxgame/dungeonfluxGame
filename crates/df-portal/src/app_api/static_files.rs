//! Serves the built account app bundle at `/app/`. The shell (`index.html`) answers `/app`,
//! `/app/` and every unknown `/app/<path>` directly with status 200 and no redirect of any
//! kind: a URL fragment (an emailed token, an invitation) survives a redirect, so a redirect
//! could carry it somewhere else. Assets are plain files with strict types; nothing outside the
//! bundle directory is ever read.

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use axum::{
    Router,
    extract::{Path as UrlPath, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use df_account_app::wire::ErrorCode;

use super::guard::ApiFailure;

const CONTENT_SECURITY_POLICY: &str = concat!(
    "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; ",
    "style-src 'self' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; ",
    "img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; ",
    "form-action 'self' https://accounts.google.com"
);
const INDEX_FILE: &str = "index.html";
const ASSETS_PREFIX: &str = "assets/";
const API_PREFIX: &str = "api";
/// A bundle file larger than this is not served; the wasm module is far below it.
const MAXIMUM_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The content type of a bundle file, by extension; `None` for anything the bundle never holds.
fn content_type(path: &str) -> Option<&'static str> {
    let (_, extension) = path.rsplit_once('.')?;
    Some(match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "webp" => "image/webp",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "json" => "application/json",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// A relative path made only of ordinary names: no `..`, no root, no drive, no backslash, no NUL.
fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0', ':'])
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && path.split('/').all(|segment| !segment.is_empty())
}

fn file_response(status: StatusCode, content_type: &'static str, bytes: Vec<u8>) -> Response {
    let mut response = (status, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

fn plain(status: StatusCode, text: &'static str) -> Response {
    file_response(
        status,
        "text/plain; charset=utf-8",
        text.as_bytes().to_vec(),
    )
}

/// Reads a regular file inside the bundle off the async threads; `None` when it is not there.
async fn read_file(root: Arc<PathBuf>, relative: String) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        let path = root.join(relative);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => return None,
        };
        if !metadata.is_file() || metadata.len() > MAXIMUM_FILE_BYTES {
            return None;
        }
        std::fs::read(path).ok()
    })
    .await
    .ok()
    .flatten()
}

async fn shell(State(root): State<Arc<PathBuf>>) -> Response {
    match read_file(root, INDEX_FILE.to_owned()).await {
        Some(bytes) => file_response(StatusCode::OK, "text/html; charset=utf-8", bytes),
        None => plain(
            StatusCode::SERVICE_UNAVAILABLE,
            "The account app is not built yet.",
        ),
    }
}

async fn file_or_shell(
    State(root): State<Arc<PathBuf>>,
    UrlPath(path): UrlPath<String>,
) -> Response {
    if path == API_PREFIX || path.starts_with("api/") {
        return ApiFailure::of(ErrorCode::NotFound).into_response();
    }
    let is_asset = path.starts_with(ASSETS_PREFIX);
    let path = path.trim_end_matches('/').to_owned();
    if !is_safe_relative(&path) {
        return plain(StatusCode::NOT_FOUND, "Not found.");
    }
    if let Some(kind) = content_type(&path)
        && let Some(bytes) = read_file(root.clone(), path.clone()).await
    {
        return file_response(StatusCode::OK, kind, bytes);
    }
    if is_asset || content_type(&path).is_some() {
        return plain(StatusCode::NOT_FOUND, "Not found.");
    }
    shell(State(root)).await
}

/// The bundle at `/app/`: the shell for every non-asset, non-API path, and the files under it.
pub fn router_for_bundle(directory: impl Into<PathBuf>) -> Router {
    Router::new()
        .route("/app", get(shell))
        .route("/app/", get(shell))
        .route("/app/{*path}", get(file_or_shell))
        .with_state(Arc::new(directory.into()))
}
