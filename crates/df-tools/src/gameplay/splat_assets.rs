//! Delivery of Gaussian splat scenes for the 3D battlefield: `/assets/splat/<file>` and
//! `/assets/splat/<bundle>/<file>` serve `assets/splat/...`.
//!
//! Splat scenes are large, so they have their own route and bounds instead of the 8 MiB generic
//! art route. Only regular files found at startup are routed (no symlinks), at most one directory
//! deep, with lowercase `[a-z0-9._-]` names and a `.splat`, `.json` or `.webp` extension, each at
//! most [`MAXIMUM_FILE_BYTES`] and at most [`MAXIMUM_FILES`] in all. The path in a request never
//! reaches the filesystem: the file list is fixed at startup and each file is read from its own
//! recorded path, re-checked against its size bound. An oversized or unreadable file refuses
//! startup like the other owned assets; an absent directory simply routes nothing.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{io, path::Path, path::PathBuf, sync::Arc, time::UNIX_EPOCH};

pub(super) const MAXIMUM_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAXIMUM_FILES: usize = 64;
const CACHE_CONTROL: &str = "public, max-age=3600";

struct Entry {
    path: PathBuf,
    mime: &'static str,
    etag: String,
}

fn mime_for(name: &str) -> Option<&'static str> {
    if name.ends_with(".splat") {
        Some("application/octet-stream")
    } else if name.ends_with(".json") {
        Some("application/json")
    } else if name.ends_with(".webp") {
        Some("image/webp")
    } else {
        None
    }
}

fn name_allowed(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

/// Lists the routable files below `directory` as `(url path, entry)`.
fn scan(directory: &Path) -> Result<Vec<(String, Entry)>, io::Error> {
    let mut found = Vec::new();
    let top = match std::fs::read_dir(directory) {
        Ok(top) => top,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error),
    };
    let mut entries: Vec<_> = top.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name_allowed(&name) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            let mut inner: Vec<_> = std::fs::read_dir(entry.path())?.collect::<Result<_, _>>()?;
            inner.sort_by_key(|file| file.file_name());
            for file in inner {
                let Some(file_name) = file.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if let Some(item) = candidate(&file.path(), &file_name)? {
                    found.push((format!("/assets/splat/{name}/{file_name}"), item));
                }
            }
        } else if let Some(item) = candidate(&entry.path(), &name)? {
            found.push((format!("/assets/splat/{name}"), item));
        }
        if found.len() > MAXIMUM_FILES {
            return Err(io::Error::other("splat file count bound refused"));
        }
    }
    Ok(found)
}

fn candidate(path: &Path, name: &str) -> Result<Option<Entry>, io::Error> {
    let Some(mime) = mime_for(name).filter(|_| name_allowed(name)) else {
        return Ok(None);
    };
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > MAXIMUM_FILE_BYTES {
        return Err(io::Error::other("owned asset bound refused"));
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_secs());
    Ok(Some(Entry {
        path: path.to_owned(),
        mime,
        etag: format!("\"{:x}-{:x}\"", metadata.len(), modified),
    }))
}

/// Routes exactly the splat files present in `directory` at startup.
pub(super) fn router(directory: &Path) -> Result<Router, io::Error> {
    let mut router = Router::new();
    for (url, entry) in scan(directory)? {
        let entry = Arc::new(entry);
        router = router.route(
            &url,
            get(move |headers: HeaderMap| {
                let entry = entry.clone();
                async move { respond(&entry, &headers).await }
            }),
        );
    }
    Ok(router)
}

async fn respond(entry: &Entry, headers: &HeaderMap) -> Response {
    let revalidated = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == entry.etag.as_bytes());
    let validators = [
        (header::ETAG, entry.etag.clone()),
        (header::CONTENT_TYPE, entry.mime.to_owned()),
        (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
    ];
    if revalidated {
        return (StatusCode::NOT_MODIFIED, validators, Body::empty()).into_response();
    }
    let within_bound = tokio::fs::metadata(&entry.path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAXIMUM_FILE_BYTES);
    if !within_bound {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(&entry.path).await {
        Ok(bytes) => {
            let length = (header::CONTENT_LENGTH, bytes.len().to_string());
            let [etag, mime, cache] = validators;
            (
                StatusCode::OK,
                [etag, mime, cache, length],
                Body::from(bytes),
            )
                .into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Method, Request};
    use tonic::codegen::Service;

    fn scratch(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("df-splat-assets-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    async fn get_path(router: &mut Router, path: &str, headers: HeaderMap) -> Response {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        *request.headers_mut() = headers;
        Service::call(router, request).await.unwrap()
    }

    #[tokio::test]
    async fn splat_files_and_one_level_bundles_are_served_and_everything_else_is_not() {
        let directory = scratch("served");
        std::fs::write(directory.join("quay.splat"), vec![7u8; 64]).unwrap();
        std::fs::create_dir_all(directory.join("pass")).unwrap();
        std::fs::write(directory.join("pass/meta.json"), b"{}").unwrap();
        std::fs::write(directory.join("pass/means_l.webp"), b"webp").unwrap();
        std::fs::write(directory.join("notes.txt"), b"not routed").unwrap();
        std::fs::write(directory.join("Quay.splat"), b"uppercase name").unwrap();
        let mut router = router(&directory).unwrap();

        let served = get_path(&mut router, "/assets/splat/quay.splat", HeaderMap::new()).await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.headers()[header::CONTENT_LENGTH], "64");
        let etag = served.headers()[header::ETAG].clone();
        let mut conditional = HeaderMap::new();
        conditional.insert(header::IF_NONE_MATCH, etag);
        let again = get_path(&mut router, "/assets/splat/quay.splat", conditional).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        let meta = get_path(
            &mut router,
            "/assets/splat/pass/meta.json",
            HeaderMap::new(),
        )
        .await;
        assert_eq!(meta.status(), StatusCode::OK);
        assert_eq!(meta.headers()[header::CONTENT_TYPE], "application/json");
        let plane = get_path(
            &mut router,
            "/assets/splat/pass/means_l.webp",
            HeaderMap::new(),
        )
        .await;
        assert_eq!(plane.headers()[header::CONTENT_TYPE], "image/webp");

        for path in [
            "/assets/splat/notes.txt",
            "/assets/splat/Quay.splat",
            "/assets/splat/../quay.splat",
            "/assets/splat/pass/other.json",
        ] {
            let response = get_path(&mut router, path, HeaderMap::new()).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn an_absent_directory_routes_nothing_and_an_oversized_file_refuses_startup() {
        let absent = std::env::temp_dir().join("df-splat-assets-absent-directory");
        assert!(router(&absent).is_ok());

        let directory = scratch("oversized");
        let file = std::fs::File::create(directory.join("big.splat")).unwrap();
        file.set_len(MAXIMUM_FILE_BYTES + 1).unwrap();
        assert!(router(&directory).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_not_routed() {
        let directory = scratch("symlink");
        std::fs::write(directory.join("real.splat"), b"data").unwrap();
        std::os::unix::fs::symlink(directory.join("real.splat"), directory.join("link.splat"))
            .unwrap();
        let listed: Vec<String> = scan(&directory)
            .unwrap()
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        assert_eq!(listed, vec!["/assets/splat/real.splat".to_owned()]);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
