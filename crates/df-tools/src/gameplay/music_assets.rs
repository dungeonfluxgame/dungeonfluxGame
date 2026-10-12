//! Delivery of the adaptive score: `/assets/music/<name>` serves `assets/music/<name>.mp3`.
//!
//! Only the fixed track names below are ever read or routed, each at most [`MAXIMUM_TRACK_BYTES`]
//! and loaded once at startup, so a request never touches the filesystem. A track that is not
//! on disk is simply not served and the browser plays without it; an oversized or unreadable
//! file refuses startup like the other owned assets.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{io, path::Path};

const MAXIMUM_TRACK_BYTES: u64 = 6 * 1024 * 1024;
const MIME: &str = "audio/mpeg";
const CACHE_CONTROL: &str = "public, max-age=3600";
/// Layers, stingers and ambient beds the music director may ask for: the main suite, one
/// alternate take per main-suite layer (`-b`), the battle (`b-`) and finale (`c-`) suites, and
/// three takes per stinger (`-2`, `-3`). Names and the player scheme live in `music_score`;
/// a file that is not on disk is simply not served and the browser plays without it.
const TRACK_NAMES: &[&str] = &[
    "score-bed",
    "score-motif",
    "score-tension",
    "score-combat",
    "score-bed-b",
    "score-motif-b",
    "score-tension-b",
    "score-combat-b",
    "b-score-bed",
    "b-score-motif",
    "b-score-tension",
    "b-score-combat",
    "c-score-bed",
    "c-score-motif",
    "c-score-tension",
    "c-score-combat",
    "sting-reveal",
    "sting-combat",
    "sting-victory",
    "sting-defeat",
    "sting-reveal-2",
    "sting-combat-2",
    "sting-victory-2",
    "sting-defeat-2",
    "sting-reveal-3",
    "sting-combat-3",
    "sting-victory-3",
    "sting-defeat-3",
    "amb-harbour",
    "amb-market",
    "amb-forest",
    "amb-mountain",
    "amb-library",
    "amb-marsh",
    "amb-graveyard",
    "amb-farm",
    "amb-night",
];

#[derive(Clone)]
struct Track {
    bytes: Bytes,
    etag: String,
}

/// Loads the allowlisted tracks found in `music_directory` and routes exactly those.
pub(super) fn router(music_directory: &Path) -> Result<Router, io::Error> {
    let mut router = Router::new();
    for name in TRACK_NAMES {
        let bytes = match super::bounded_asset(
            music_directory.join(format!("{name}.mp3")),
            MAXIMUM_TRACK_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let track = Track {
            etag: format!("\"{:x}\"", Sha256::digest(&bytes)),
            bytes,
        };
        router = router.route(
            &format!("/assets/music/{name}"),
            get(move |headers: HeaderMap| {
                let track = track.clone();
                async move { respond(&track, &headers) }
            }),
        );
    }
    Ok(router)
}

fn respond(track: &Track, headers: &HeaderMap) -> Response {
    let revalidated = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == track.etag.as_bytes());
    let headers = [
        (header::ETAG, track.etag.clone()),
        (header::CONTENT_TYPE, MIME.to_owned()),
        (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
        (header::CONTENT_LENGTH, track.bytes.len().to_string()),
    ];
    if revalidated {
        return (StatusCode::NOT_MODIFIED, headers, Body::empty()).into_response();
    }
    (StatusCode::OK, headers, Body::from(track.bytes.clone())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Method, Request};
    use tonic::codegen::Service;

    fn scratch(label: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("df-music-assets-{label}-{}", std::process::id()));
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
    async fn music_track_is_served_as_cached_audio_and_missing_tracks_are_not() {
        let directory = scratch("served");
        std::fs::write(directory.join("score-bed.mp3"), b"mp3 bytes").unwrap();
        let mut router = router(&directory).unwrap();

        let served = get_path(&mut router, "/assets/music/score-bed", HeaderMap::new()).await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.headers()[header::CONTENT_TYPE], MIME);
        assert_eq!(served.headers()[header::CACHE_CONTROL], CACHE_CONTROL);
        assert_eq!(served.headers()[header::CONTENT_LENGTH], "9");
        let etag = served.headers()[header::ETAG].clone();

        let mut conditional = HeaderMap::new();
        conditional.insert(header::IF_NONE_MATCH, etag);
        let revalidated = get_path(&mut router, "/assets/music/score-bed", conditional).await;
        assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);

        let absent = get_path(&mut router, "/assets/music/score-combat", HeaderMap::new()).await;
        assert_eq!(absent.status(), StatusCode::NOT_FOUND);
        let unlisted = get_path(&mut router, "/assets/music/other", HeaderMap::new()).await;
        assert_eq!(unlisted.status(), StatusCode::NOT_FOUND);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn alternate_and_suite_tracks_are_served_when_present_and_skipped_when_absent() {
        let directory = scratch("alternates");
        std::fs::write(directory.join("score-motif-b.mp3"), b"alternate take").unwrap();
        std::fs::write(directory.join("b-score-bed.mp3"), b"battle suite").unwrap();
        let mut router = router(&directory).unwrap();

        let alternate =
            get_path(&mut router, "/assets/music/score-motif-b", HeaderMap::new()).await;
        assert_eq!(alternate.status(), StatusCode::OK);
        let suite = get_path(&mut router, "/assets/music/b-score-bed", HeaderMap::new()).await;
        assert_eq!(suite.status(), StatusCode::OK);
        let not_yet_made =
            get_path(&mut router, "/assets/music/c-score-bed", HeaderMap::new()).await;
        assert_eq!(not_yet_made.status(), StatusCode::NOT_FOUND);
        let base_still_served =
            get_path(&mut router, "/assets/music/score-bed", HeaderMap::new()).await;
        assert_eq!(base_still_served.status(), StatusCode::NOT_FOUND);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn music_directory_that_is_absent_routes_nothing_and_an_oversized_track_refuses_startup() {
        let absent = std::env::temp_dir().join("df-music-assets-absent-directory");
        assert!(router(&absent).is_ok());

        let directory = scratch("oversized");
        let file = std::fs::File::create(directory.join("amb-night.mp3")).unwrap();
        file.set_len(MAXIMUM_TRACK_BYTES + 1).unwrap();
        assert!(router(&directory).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
