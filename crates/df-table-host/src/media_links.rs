//! Signed links to media for phones on the player listener (owner, 2026-11-16).
//!
//! A phone sees only the media the host's answers to it name in their picture fields: its own
//! hero's pack, the plates and close-ups shown to the table, the pictures and sounds of its pages
//! and its frame. Every JSON answer of a phone's own routes (`/play/..` and its frame) carries, in
//! the `path::MEDIA_LINKS_HEADER` header, a short-lived signed link for each library file a
//! picture field of the answer names (`MEDIA_FIELDS`: what the host put there, never a player's
//! words: a note or a name that spells a file name gets no link), and the player listener serves a
//! media file only with a link that is signed for it and not expired (`path::MEDIA_LINK_PARAM`).
//! A fixture and the other files of the page are signed for nothing. A link is
//! `<expiry, unix seconds>.<HMAC-SHA256 of the name and expiry, hex>` under a secret the host
//! draws at start, so links die with the host. The loopback dev listener serves media unsigned.
//!
//! Lines: `HOST media_refused reason=unsigned|bad_signature|expired`.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use df_playground::media::{is_plain_file_name, media_kind};
use df_playground::path::{MEDIA_LINKS_HEADER, PLAY};
use df_playground::seat::wire::hex_digits;
use df_table::ports::Clock;
use serde::Deserialize;
use serde_json::Value;

use crate::limits::{MAXIMUM_RESPONSE_BYTES, MEDIA_LINK_LIFETIME};
use crate::media_library::media_directory;
use crate::observe::WallClock;
use crate::state::AppState;
use crate::wire::route;

/// The fields of an answer that hold a media library file the host chose: a picture, a plate, a
/// portrait, a pack item's picture, a sound or an image's media. Any other string, a player's own
/// words among them, is never signed.
const MEDIA_FIELDS: [&str; 4] = ["picture", "plate", "portrait", "media"];

/// The domain tag of a media link's signature.
const LINK_TAG: &str = "df-table media link";

/// The host's signer of media links: a secret drawn at start. Without one (the system gave no
/// randomness) no link is signed and the player listener serves no media.
#[derive(Debug)]
pub(crate) struct MediaLinks {
    secret: Option<[u8; 32]>,
}

/// Unix seconds now, from the host's wall clock (read at the edge).
fn now_seconds() -> u64 {
    WallClock.now_ms() / 1_000
}

impl MediaLinks {
    pub(crate) fn new() -> Self {
        Self {
            secret: df_campaign::secrets::random_bytes::<32>().ok(),
        }
    }

    fn signature(&self, name: &str, expires: u64) -> Option<String> {
        let secret = self.secret.as_ref()?;
        let signed = format!("{name}\n{expires}");
        Some(hex_digits(&df_campaign::secrets::keyed_digest(
            secret,
            LINK_TAG,
            signed.as_bytes(),
        )))
    }

    /// A link to `name` that holds for `MEDIA_LINK_LIFETIME` from `now` (unix seconds).
    pub(crate) fn link(&self, name: &str, now: u64) -> Option<String> {
        let expires = now.saturating_add(MEDIA_LINK_LIFETIME.as_secs());
        Some(format!("{expires}.{}", self.signature(name, expires)?))
    }

    /// Whether `link` is signed for `name` and not expired at `now`; why not when it is not.
    pub(crate) fn check(&self, name: &str, link: &str, now: u64) -> Result<(), &'static str> {
        let (expires, given) = link.split_once('.').ok_or("bad_signature")?;
        let expires: u64 = expires.parse().map_err(|_| "bad_signature")?;
        let wanted = self.signature(name, expires).ok_or("bad_signature")?;
        // Compared whole, byte by byte, so the time taken says nothing of where they differ.
        let differ = wanted.len() != given.len()
            || wanted
                .bytes()
                .zip(given.bytes())
                .fold(0u8, |seen, (left, right)| seen | (left ^ right))
                != 0;
        if differ {
            return Err("bad_signature");
        }
        if expires < now {
            return Err("expired");
        }
        Ok(())
    }
}

/// Every media file of the library kind (a picture, a sound, a video) that a picture field of
/// `value` (`MEDIA_FIELDS`, at any depth) names, each once. A string under any other field is
/// never read, however much it looks like a file name.
pub(crate) fn named_media(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Array(items) => items.iter().for_each(|item| named_media(item, found)),
        Value::Object(fields) => {
            for (key, field) in fields {
                if MEDIA_FIELDS.contains(&key.as_str()) {
                    field_media(field, found);
                } else {
                    named_media(field, found);
                }
            }
        }
        _ => {}
    }
}

/// The media file a picture field holds: its text, or, for a field holding a picture's details
/// (an image with its caption, a pack item), the picture fields inside it.
fn field_media(field: &Value, found: &mut Vec<String>) {
    match field {
        Value::String(text) => {
            if is_plain_file_name(text) && media_kind(text).is_some() && !found.contains(text) {
                found.push(text.clone());
            }
        }
        other => named_media(other, found),
    }
}

/// Whether the answer to `path` is a phone's own (`/play/..` or its frame), whose pictures it is
/// given links to. A fixture, the theme and the other files of the page are signed for nothing.
fn signs_answers_of(path: &str) -> bool {
    path == route::PLAYGROUND_FRAME
        || path
            .strip_prefix(PLAY)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// The player listener's middleware: a JSON answer of a phone's own routes gets a signed link
/// for each library file its picture fields name (`MEDIA_LINKS_HEADER`, `name=link;..`). What the
/// host put in those fields is what the phone may see.
pub(crate) async fn sign_answers(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let signs = signs_answers_of(request.uri().path());
    let response = next.run(request).await;
    if !signs {
        return response;
    }
    let json = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if !json {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAXIMUM_RESPONSE_BYTES).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut names = Vec::new();
    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        named_media(&value, &mut names);
    }
    let media = media_directory(state.playground().files());
    let now = now_seconds();
    let links: Vec<String> = names
        .into_iter()
        .filter(|name| media.join(name).is_file())
        .filter_map(|name| {
            let link = state.media_links().link(&name, now)?;
            Some(format!("{name}={link}"))
        })
        .collect();
    if !links.is_empty()
        && let Ok(value) = HeaderValue::from_str(&links.join(";"))
    {
        parts.headers.insert(MEDIA_LINKS_HEADER, value);
    }
    Response::from_parts(parts, Body::from(bytes))
}

/// The signed link of a media request (`path::MEDIA_LINK_PARAM`).
#[derive(Deserialize)]
pub(crate) struct LinkQuery {
    #[serde(rename = "ml")]
    link: Option<String>,
}

/// `GET path::MEDIA_FILE` on the player listener: the file, only with a link signed for it.
pub(crate) async fn signed_media_file(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
    Query(query): Query<LinkQuery>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let checked = match &query.link {
        Some(link) => state.media_links().check(&name, link, now_seconds()),
        None => Err("unsigned"),
    };
    if let Err(reason) = checked {
        state.record_line(format!("HOST media_refused reason={reason}"));
        return StatusCode::NOT_FOUND.into_response();
    }
    crate::media_library::media_file(State(state), UrlPath(name), method, headers).await
}

#[cfg(test)]
mod media_links_tests;
