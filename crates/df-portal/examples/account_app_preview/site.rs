//! Serves the public static site at the root of the local account preview.

use std::{
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use axum::{
    extract::Extension,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};

fn relative_file(path: &str) -> Option<PathBuf> {
    if path.contains(['\\', '\0', ':']) {
        return None;
    }
    let path = path.strip_prefix('/')?;
    let path = if path.is_empty() {
        "index.html".to_owned()
    } else if path.ends_with('/') {
        format!("{path}index.html")
    } else {
        path.to_owned()
    };
    if Path::new(&path)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
        || path.split('/').any(str::is_empty)
    {
        return None;
    }
    Some(PathBuf::from(path))
}

fn content_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "html" => Some("text/html; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "md" => Some("text/plain; charset=utf-8"),
        "webp" => Some("image/webp"),
        "webm" => Some("video/webm"),
        _ => None,
    }
}

async fn read_file(root: Arc<PathBuf>, relative: PathBuf) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        let path = std::fs::canonicalize(root.join(relative)).ok()?;
        if !path.starts_with(root.as_path()) || !path.is_file() {
            return None;
        }
        std::fs::read(path).ok()
    })
    .await
    .ok()
    .flatten()
}

pub(crate) async fn fallback(Extension(root): Extension<Arc<PathBuf>>, uri: Uri) -> Response {
    let Some(relative) = relative_file(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(kind) = content_type(&relative) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(bytes) = read_file(root, relative).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

#[cfg(test)]
mod tests {
    use super::relative_file;

    #[test]
    fn relative_file_serves_root_and_nested_indexes() {
        assert_eq!(relative_file("/"), Some("index.html".into()));
        assert_eq!(
            relative_file("/pricing/"),
            Some("pricing/index.html".into())
        );
    }

    #[test]
    fn relative_file_refuses_path_traversal() {
        assert_eq!(relative_file("/../../Cargo.toml"), None);
        assert_eq!(relative_file("/..\\Cargo.toml"), None);
        assert_eq!(relative_file("/C:/Cargo.toml"), None);
    }
}
