//! Delivery of Gaussian splat scenes for the 3D battlefield: `/assets/splat/<file>` and
//! `/assets/splat/<dir>/.../<file>` serve the splat directory (`assets/splat` by default, or
//! `DF_GAMEPLAY_SPLAT_DIR`), including the nested `<scene>/v1/<level>_<chunk>/<plane>` layout of
//! a streamed level-of-detail scan.
//!
//! Splat scenes are large, so they have their own route and bounds instead of the 8 MiB generic
//! art route. Only regular files found at startup are routed (no symlinks), at most
//! [`MAXIMUM_DEPTH`] directories deep, with lowercase `[a-z0-9._-]` names and a `.splat`, `.ply`,
//! `.sog`, `.json` or `.webp` extension, each at most [`MAXIMUM_FILE_BYTES`] and at most
//! [`MAXIMUM_FILES`] in all.
//! The path in a request never reaches the filesystem: it is only a key into the table built at
//! startup, and each file is read from its own recorded path, re-checked against its size bound.
//! An oversized, unreadable or too numerous set refuses startup like the other owned assets; an
//! absent directory simply routes nothing.
use axum::{
    Router,
    body::Body,
    extract::{Path as RoutePath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{collections::HashMap, io, path::Path, path::PathBuf, sync::Arc, time::UNIX_EPOCH};

pub(super) const MAXIMUM_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAXIMUM_FILES: usize = 4096;
/// Directories below the splat directory: `<scene>/v1/<level>_<chunk>`.
pub(super) const MAXIMUM_DEPTH: usize = 3;
const CACHE_CONTROL: &str = "public, max-age=3600";

struct Entry {
    root: PathBuf,
    path: PathBuf,
    mime: &'static str,
    etag: String,
}

fn mime_for(name: &str) -> Option<&'static str> {
    if name.ends_with(".splat") || name.ends_with(".ply") || name.ends_with(".sog") {
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
    match std::fs::read_dir(directory) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(found),
        Err(error) => return Err(error),
    }
    let root = std::fs::canonicalize(directory)?;
    walk(&root, &root, "/assets/splat", 0, &mut found)?;
    Ok(found)
}

fn walk(
    root: &Path,
    directory: &Path,
    url: &str,
    depth: usize,
    found: &mut Vec<(String, Entry)>,
) -> Result<(), io::Error> {
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<Result<_, _>>()?;
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
            if depth < MAXIMUM_DEPTH {
                walk(
                    root,
                    &entry.path(),
                    &format!("{url}/{name}"),
                    depth + 1,
                    found,
                )?;
            }
        } else if let Some(item) = candidate(root, &entry.path(), &name)? {
            found.push((format!("{url}/{name}"), item));
        }
        if found.len() > MAXIMUM_FILES {
            return Err(io::Error::other("splat file count bound refused"));
        }
    }
    Ok(())
}

fn candidate(root: &Path, path: &Path, name: &str) -> Result<Option<Entry>, io::Error> {
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
        root: root.to_owned(),
        path: path.to_owned(),
        mime,
        etag: format!("\"{:x}-{:x}\"", metadata.len(), modified),
    }))
}

type Table = Arc<HashMap<String, Entry>>;

/// Routes exactly the splat files present in `directory` at startup.
pub(super) fn router(directory: &Path) -> Result<Router, io::Error> {
    let table: Table = Arc::new(scan(directory)?.into_iter().collect());
    Ok(Router::new()
        .route("/assets/splat/{*path}", get(lookup))
        .with_state(table))
}

async fn lookup(
    State(table): State<Table>,
    RoutePath(path): RoutePath<String>,
    headers: HeaderMap,
) -> Response {
    match table.get(&format!("/assets/splat/{path}")) {
        Some(entry) => respond(entry, &headers).await,
        None => StatusCode::NOT_FOUND.into_response(),
    }
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
    let Some(path) = contained_file(&entry.root, &entry.path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let within_bound = tokio::fs::metadata(&path)
        .await
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= MAXIMUM_FILE_BYTES);
    if !within_bound {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(&path).await {
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

fn contained_file(root: &Path, path: &Path) -> Option<PathBuf> {
    let resolved = std::fs::canonicalize(path).ok()?;
    is_within_root(root, &resolved).then_some(resolved)
}

fn is_within_root(root: &Path, resolved: &Path) -> bool {
    resolved.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Method, Request};
    use tonic::codegen::Service;

    fn scratch(label: &str) -> PathBuf {
        let lane_directory = std::env::current_dir()
            .unwrap()
            .join("artifacts/tmp/lanes/M6");
        std::fs::create_dir_all(&lane_directory).unwrap();
        let directory =
            lane_directory.join(format!("df-splat-assets-{label}-{}", std::process::id()));
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
    async fn splat_files_and_nested_scan_layouts_are_served_and_everything_else_is_not() {
        let directory = scratch("served");
        std::fs::write(directory.join("quay.splat"), vec![7u8; 64]).unwrap();
        std::fs::write(directory.join("world.ply"), b"ply-data").unwrap();
        std::fs::write(directory.join("world.sog"), b"sog-data").unwrap();
        std::fs::create_dir_all(directory.join("pass")).unwrap();
        std::fs::write(directory.join("pass/meta.json"), b"{}").unwrap();
        std::fs::write(directory.join("pass/means_l.webp"), b"webp").unwrap();
        std::fs::create_dir_all(directory.join("9b6bc6bf/v1/1_0")).unwrap();
        std::fs::write(directory.join("9b6bc6bf/v1/lod-meta.json"), b"{}").unwrap();
        std::fs::write(directory.join("9b6bc6bf/v1/1_0/means_l.webp"), b"webp").unwrap();
        std::fs::create_dir_all(directory.join("a/b/c/d")).unwrap();
        std::fs::write(directory.join("a/b/c/d/too-deep.json"), b"{}").unwrap();
        std::fs::write(directory.join("notes.txt"), b"not routed").unwrap();
        std::fs::write(directory.join("Upper.splat"), b"uppercase name").unwrap();
        let mut router = router(&directory).unwrap();

        let served = get_path(&mut router, "/assets/splat/quay.splat", HeaderMap::new()).await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.headers()[header::CONTENT_LENGTH], "64");
        let etag = served.headers()[header::ETAG].clone();
        let mut conditional = HeaderMap::new();
        conditional.insert(header::IF_NONE_MATCH, etag);
        let again = get_path(&mut router, "/assets/splat/quay.splat", conditional).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

        for path in ["/assets/splat/world.ply", "/assets/splat/world.sog"] {
            let response = get_path(&mut router, path, HeaderMap::new()).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/octet-stream"
            );
        }

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
            "/assets/splat/9b6bc6bf/v1/lod-meta.json",
            "/assets/splat/9b6bc6bf/v1/1_0/means_l.webp",
        ] {
            let response = get_path(&mut router, path, HeaderMap::new()).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }

        for path in [
            "/assets/splat/notes.txt",
            "/assets/splat/world.spz",
            "/assets/splat/Upper.splat",
            "/assets/splat/../quay.splat",
            "/assets/splat/pass/other.json",
            "/assets/splat/9b6bc6bf/v1/../../quay.splat",
            "/assets/splat/9b6bc6bf/v1/1_0/../../../quay.splat",
            "/assets/splat/%2e%2e/quay.splat",
            "/assets/splat/9b6bc6bf//v1/lod-meta.json",
            "/assets/splat/a/b/c/d/too-deep.json",
            "/assets/splat/9b6bc6bf/v1/1_0",
            "/assets/splat/",
        ] {
            let response = get_path(&mut router, path, HeaderMap::new()).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn an_absent_directory_routes_nothing_and_oversized_supported_files_refuse_startup() {
        let absent = std::env::current_dir()
            .unwrap()
            .join("artifacts/tmp/lanes/M6/df-splat-assets-absent-directory");
        assert!(router(&absent).is_ok());

        for extension in ["splat", "ply", "sog"] {
            let directory = scratch(&format!("oversized-{extension}"));
            let file = std::fs::File::create(directory.join(format!("big.{extension}"))).unwrap();
            file.set_len(MAXIMUM_FILE_BYTES + 1).unwrap();
            assert!(router(&directory).is_err(), "{extension}");
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn containment_requires_the_resolved_path_to_remain_under_the_root() {
        let root = Path::new("/configured/splats");
        assert!(is_within_root(
            root,
            Path::new("/configured/splats/map.splat")
        ));
        assert!(!is_within_root(root, Path::new("/configured/private.txt")));
    }

    #[test]
    fn more_files_than_the_bound_refuses_startup() {
        let directory = scratch("many");
        for index in 0..=MAXIMUM_FILES {
            std::fs::write(directory.join(format!("f{index}.json")), b"{}").unwrap();
        }
        assert!(router(&directory).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_or_directory_is_not_routed() {
        let directory = scratch("symlink");
        std::fs::write(directory.join("real.splat"), b"data").unwrap();
        std::os::unix::fs::symlink(directory.join("real.splat"), directory.join("link.splat"))
            .unwrap();
        std::fs::create_dir_all(directory.join("real/v1")).unwrap();
        std::fs::write(directory.join("real/v1/meta.json"), b"{}").unwrap();
        std::os::unix::fs::symlink(directory.join("real"), directory.join("alias")).unwrap();
        let mut listed: Vec<String> = scan(&directory)
            .unwrap()
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            vec![
                "/assets/splat/real.splat".to_owned(),
                "/assets/splat/real/v1/meta.json".to_owned()
            ]
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_replaced_by_an_outside_symlink_is_not_served() {
        let parent = scratch("replaced-link");
        let directory = parent.join("splat");
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("map.splat");
        let outside = parent.join("outside.splat");
        std::fs::write(&file, b"admitted bytes").unwrap();
        std::fs::write(&outside, b"outside bytes").unwrap();
        let mut router = router(&directory).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();

        let response = get_path(&mut router, "/assets/splat/map.splat", HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        std::fs::remove_dir_all(parent).unwrap();
    }
}
