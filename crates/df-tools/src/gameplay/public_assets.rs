//! Delivery for the fixed public illustrations loaded within the startup byte bounds.
//! Private/generated media still requires df-assets' current metadata/access authority;
//! these public immutable snapshots neither register nor authorize that media.

#[cfg(test)]
mod tests;

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::convert::Infallible;

const MAX_RANGE_HEADER_BYTES: usize = 128;
const MAX_CONDITIONAL_HEADER_BYTES: usize = 4096;
const MAX_BODY_CHUNK_BYTES: usize = 65536;
const ART_PATH: &str = "/assets/ui/scenes/mara-harbor-v4.png";
const CAMPFIRE_PATH: &str = "/assets/concept-art/scene-campfire-under-stars.webp";
const PORTRAIT_PATH: &str = "/assets/concept-art/vell-avatar.webp";
const INN_PATH: &str = "/assets/concept-art/scene-tavern-barkeep-talk-rain.webp";

#[derive(Clone)]
pub(super) struct PublicAssets {
    pub(super) art: Bytes,
    pub(super) campfire: Bytes,
    pub(super) portrait: Bytes,
    pub(super) inn: Option<Bytes>,
}

#[derive(Clone)]
struct PublicSnapshot {
    bytes: Bytes,
    etag: String,
}

impl PublicSnapshot {
    fn new(bytes: Bytes) -> Self {
        // Hash the exact immutable startup bytes once, never a mutable file/path.
        let etag = format!("\"{:x}\"", Sha256::digest(&bytes));
        Self { bytes, etag }
    }
}

#[derive(Clone)]
struct PublicSnapshots {
    art: PublicSnapshot,
    campfire: PublicSnapshot,
    portrait: PublicSnapshot,
    inn: Option<PublicSnapshot>,
}

pub(super) fn router(assets: PublicAssets) -> Router {
    let snapshots = PublicSnapshots {
        art: PublicSnapshot::new(assets.art),
        campfire: PublicSnapshot::new(assets.campfire),
        portrait: PublicSnapshot::new(assets.portrait),
        inn: assets.inn.map(PublicSnapshot::new),
    };
    Router::new()
        .route(ART_PATH, get(art))
        .route(CAMPFIRE_PATH, get(campfire))
        .route(PORTRAIT_PATH, get(portrait))
        .route(INN_PATH, get(inn))
        .with_state(snapshots)
}

/// One allowlisted optional public file, already loaded within the startup byte bounds.
pub(super) struct ExtraAsset {
    pub(super) url: String,
    pub(super) mime: &'static str,
    pub(super) bytes: Bytes,
}

/// Routes exactly the loaded allowlist; every other path stays unrouted.
pub(super) fn extra_router(assets: Vec<ExtraAsset>) -> Router {
    let mut router = Router::new();
    for asset in assets {
        let snapshot = PublicSnapshot::new(asset.bytes);
        let mime = asset.mime;
        router = router.route(
            &asset.url,
            get(move |method: Method, headers: HeaderMap| {
                let snapshot = snapshot.clone();
                async move { response(snapshot, mime, &method, &headers) }
            }),
        );
    }
    router
}

async fn art(
    State(assets): State<PublicSnapshots>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    response(assets.art, "image/png", &method, &headers)
}

async fn campfire(
    State(assets): State<PublicSnapshots>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    response(assets.campfire, "image/webp", &method, &headers)
}

async fn portrait(
    State(assets): State<PublicSnapshots>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    response(assets.portrait, "image/webp", &method, &headers)
}

async fn inn(
    State(assets): State<PublicSnapshots>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    match assets.inn {
        Some(bytes) => response(bytes, "image/webp", &method, &headers),
        None => Redirect::temporary(ART_PATH).into_response(),
    }
}

enum Selection {
    Full,
    Partial { start: usize, end: usize },
    Unsatisfiable,
}

fn selection(headers: &HeaderMap, length: usize, etag: &str) -> Selection {
    // Weak, stale, malformed or date validators cannot resume these exact bytes.
    if headers.contains_key(header::IF_RANGE) && !if_range_matches(headers, etag) {
        return Selection::Full;
    }
    let mut values = headers.get_all(header::RANGE).iter();
    let Some(value) = values.next() else {
        return Selection::Full;
    };
    if values.next().is_some() || value.as_bytes().len() > MAX_RANGE_HEADER_BYTES {
        return Selection::Full;
    }
    let Some((unit, value)) = value
        .to_str()
        .ok()
        .and_then(|value| value.trim().split_once('='))
    else {
        return Selection::Full;
    };
    if !unit.eq_ignore_ascii_case("bytes") {
        return Selection::Full;
    }
    let Some((start, end)) = value.trim().split_once('-') else {
        return Selection::Full;
    };
    let length = length as u64;
    if start.is_empty() {
        let Some(suffix) = decimal(end) else {
            return Selection::Full;
        };
        return if suffix == 0 || length == 0 {
            Selection::Unsatisfiable
        } else {
            Selection::Partial {
                start: length.saturating_sub(suffix) as usize,
                end: length as usize,
            }
        };
    }
    let start_text = start;
    let Some(start) = decimal(start_text) else {
        return Selection::Full;
    };
    let end = if end.is_empty() {
        u64::MAX
    } else {
        let Some(end_value) = decimal(end) else {
            return Selection::Full;
        };
        let start_text = start_text.trim_start_matches('0');
        let end_text = end.trim_start_matches('0');
        if (start_text.len(), start_text) > (end_text.len(), end_text) {
            return Selection::Full;
        }
        end_value
    };
    if start >= length {
        return Selection::Unsatisfiable;
    }
    Selection::Partial {
        start: start as usize,
        end: end.saturating_add(1).min(length) as usize,
    }
}

fn trim_ows(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn if_range_matches(headers: &HeaderMap, etag: &str) -> bool {
    let mut values = headers.get_all(header::IF_RANGE).iter();
    let Some(value) = values.next() else {
        return false;
    };
    values.next().is_none()
        && value.as_bytes().len() <= MAX_CONDITIONAL_HEADER_BYTES
        && trim_ows(value.as_bytes()) == etag.as_bytes()
}

#[derive(Clone, Copy)]
enum TagComparison {
    Strong,
    Weak,
}

fn entity_tag_condition_matches(
    headers: &HeaderMap,
    name: header::HeaderName,
    etag: &str,
    comparison: TagComparison,
) -> Option<bool> {
    let values = headers.get_all(name);
    let mut count = 0usize;
    let mut bytes = 0usize;
    for value in values.iter() {
        // Count separators as well, so repeated empty fields are bounded too.
        bytes = bytes
            .saturating_add(value.as_bytes().len())
            .saturating_add(1);
        if bytes > MAX_CONDITIONAL_HEADER_BYTES {
            return None;
        }
        count += 1;
    }
    if count == 0 {
        return None;
    }
    if count == 1
        && values
            .iter()
            .next()
            .is_some_and(|value| trim_ows(value.as_bytes()) == b"*")
    {
        return Some(true);
    }
    let mut matched = false;
    for value in values.iter() {
        // Ignore a malformed field as a whole, never accepting an earlier match.
        // None also explicitly represents an absent or oversized condition.
        matched |= entity_tag_list_matches(value.as_bytes(), etag.as_bytes(), comparison)?;
    }
    Some(matched)
}

fn entity_tag_list_matches(
    mut value: &[u8],
    etag: &[u8],
    comparison: TagComparison,
) -> Option<bool> {
    let mut matched = false;
    value = trim_ows(value);
    while !value.is_empty() {
        // HTTP list syntax allows empty elements. Commas within quoted opaque
        // tags are ordinary bytes, so splitting this field on commas is unsafe.
        if value.first() == Some(&b',') {
            value = trim_ows(&value[1..]);
            continue;
        }
        let weak = value.starts_with(b"W/");
        if weak {
            value = &value[2..];
        }
        if value.first() != Some(&b'"') {
            return None;
        }
        let end = value.get(1..)?.iter().position(|byte| *byte == b'"')? + 1;
        let opaque = value.get(1..end)?;
        if !opaque
            .iter()
            .all(|byte| matches!(*byte, b'!' | b'#'..=b'~' | 0x80..=0xff))
        {
            return None;
        }
        matched |=
            (!weak || matches!(comparison, TagComparison::Weak)) && value.get(..=end)? == etag;
        value = trim_ows(value.get(end + 1..)?);
        if !value.is_empty() && value.first() != Some(&b',') {
            return None;
        }
    }
    Some(matched)
}

fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // A syntactically valid HTTP decimal can exceed u64. Saturation preserves clipping
    // and suffix semantics without overflow or allocating for an untrusted integer.
    Some(value.parse().unwrap_or(u64::MAX))
}

fn response(
    snapshot: PublicSnapshot,
    mime: &'static str,
    method: &Method,
    headers: &HeaderMap,
) -> Response {
    let PublicSnapshot { bytes, etag } = snapshot;
    let length = bytes.len();
    // RFC 9110 section 13.2.2: If-Match precedes If-None-Match and Range.
    // These immutable snapshots have no Last-Modified timestamp authority.
    if entity_tag_condition_matches(headers, header::IF_MATCH, &etag, TagComparison::Strong)
        == Some(false)
    {
        return (
            StatusCode::PRECONDITION_FAILED,
            [
                (header::ETAG, etag),
                (header::CONTENT_TYPE, mime.to_owned()),
                (header::ACCEPT_RANGES, "bytes".to_owned()),
                (header::CONTENT_LENGTH, "0".to_owned()),
            ],
            Body::empty(),
        )
            .into_response();
    }
    if entity_tag_condition_matches(headers, header::IF_NONE_MATCH, &etag, TagComparison::Weak)
        == Some(true)
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CONTENT_TYPE, mime.to_owned()),
                (header::ACCEPT_RANGES, "bytes".to_owned()),
                // A 304 length, when present, describes the complete selected
                // representation, never the zero-byte response body.
                (header::CONTENT_LENGTH, length.to_string()),
            ],
            Body::empty(),
        )
            .into_response();
    }
    // HTTP Range applies to GET; HEAD describes the full representation.
    let selection = if method == Method::GET {
        selection(headers, length, &etag)
    } else {
        Selection::Full
    };
    let (status, bytes, content_range) = match selection {
        Selection::Full => (StatusCode::OK, bytes, None),
        Selection::Partial { start, end } => (
            StatusCode::PARTIAL_CONTENT,
            bytes.slice(start..end),
            Some(format!("bytes {start}-{}/{length}", end - 1)),
        ),
        Selection::Unsatisfiable => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            Bytes::new(),
            Some(format!("bytes */{length}")),
        ),
    };
    let content_length = bytes.len();
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        // Poll-driven zero-copy chunks own only the bounded startup snapshot. Dropping
        // the body cancels delivery immediately; there is no worker or read-ahead queue.
        Body::from_stream(futures::stream::unfold(bytes, |mut remaining| async move {
            if remaining.is_empty() {
                return None;
            }
            let chunk = remaining.split_to(remaining.len().min(MAX_BODY_CHUNK_BYTES));
            Some((Ok::<_, Infallible>(chunk), remaining))
        }))
    };
    let mut response = (
        status,
        [
            (header::ETAG, etag),
            (header::CONTENT_TYPE, mime.to_owned()),
            (header::ACCEPT_RANGES, "bytes".to_owned()),
            (header::CONTENT_LENGTH, content_length.to_string()),
        ],
        body,
    )
        .into_response();
    if let Some(content_range) = content_range {
        let Ok(value) = content_range.parse() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response
}
