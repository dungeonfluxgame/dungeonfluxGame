//! The join link players open, and the QR code of that link at `/gameplay/join-qr.svg`.
//!
//! The base address is `DF_GAMEPLAY_PUBLIC_URL` when it is set, or else the listener's own
//! address (see `startup_configuration::session_settings`). The link is
//! `<base>/gameplay/player?room=<code>`. A room code is letters and digits only, so it needs no
//! escaping. The QR code is encoded here and written out as an SVG of dark modules on a light
//! quiet zone, so the browser draws it without scripts or image decoding.
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use qrcode::{Color, EcLevel, QrCode, types::QrError};
use sha2::{Digest, Sha256};
use std::{
    io,
    sync::{Arc, OnceLock},
};

/// The route the join screen's image and the startup line name.
pub(super) const QR_PATH: &str = "/gameplay/join-qr.svg";
const PLAYER_PATH: &str = "/gameplay/player";
const MIME: &str = "image/svg+xml";
/// Revalidated by entity tag, so a restarted run with another room never shows a stale code.
const CACHE_CONTROL: &str = "no-cache";
/// The qualification listener's address. Startup installs the real base before any view is
/// built, so this answers only where no server has started (tests and tools).
const FALLBACK_ORIGIN: &str = "http://127.0.0.1:63309";
/// Light modules around the code on every side; the QR specification asks for at least four.
const QUIET_ZONE: usize = 4;

static JOIN_ORIGIN: OnceLock<String> = OnceLock::new();

/// Fixes the base address of the process, once, before any view or the QR code is built.
pub(super) fn install(origin: &str) -> Result<(), io::Error> {
    JOIN_ORIGIN
        .set(origin.to_owned())
        .map_err(|_| io::Error::other("join origin already installed"))
}

/// The join link of `room_code` at the installed base address.
pub(super) fn join_url(room_code: &str) -> String {
    link(
        JOIN_ORIGIN.get().map_or(FALLBACK_ORIGIN, String::as_str),
        room_code,
    )
}

fn link(origin: &str, room_code: &str) -> String {
    format!("{origin}{PLAYER_PATH}?room={room_code}")
}

/// The QR code of one join link as an SVG document, with the entity tag that names it.
pub(super) struct JoinQr {
    svg: String,
    etag: String,
}

impl JoinQr {
    /// The code of the join link for `room_code` at the installed base address.
    pub(super) fn for_room(room_code: &str) -> Result<Self, QrError> {
        Self::new(&join_url(room_code))
    }

    /// The code of `url`, which is encoded at medium error correction.
    pub(super) fn new(url: &str) -> Result<Self, QrError> {
        let svg = svg(url)?;
        let etag = format!("\"{:x}\"", Sha256::digest(svg.as_bytes()));
        Ok(Self { svg, etag })
    }
}

/// One unit square per dark module, offset by the quiet zone, all drawn clockwise in a single
/// path so renderers fill the code as one shape without seams between neighbouring modules.
fn svg(url: &str) -> Result<String, QrError> {
    let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::M)?;
    let width = code.width();
    let side = width + 2 * QUIET_ZONE;
    let mut modules = String::new();
    for (index, color) in code.into_colors().into_iter().enumerate() {
        if color == Color::Dark {
            let (x, y) = (index % width + QUIET_ZONE, index / width + QUIET_ZONE);
            modules.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" \
         shape-rendering=\"crispEdges\" role=\"img\" aria-label=\"QR code for the join link\">\
         <rect width=\"{side}\" height=\"{side}\" fill=\"#ffffff\"/>\
         <path fill=\"#000000\" d=\"{modules}\"/></svg>"
    ))
}

/// Serves the one QR code; the route answers `304 Not Modified` to its current entity tag.
pub(super) fn router(qr: JoinQr) -> Router {
    let qr = Arc::new(qr);
    Router::new().route(
        QR_PATH,
        get(move |headers: HeaderMap| {
            let qr = qr.clone();
            async move { respond(&qr, &headers) }
        }),
    )
}

fn respond(qr: &JoinQr, headers: &HeaderMap) -> Response {
    let revalidated = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == qr.etag.as_bytes());
    let headers = [
        (header::ETAG, qr.etag.clone()),
        (header::CONTENT_TYPE, MIME.to_owned()),
        (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
        (header::CONTENT_LENGTH, qr.svg.len().to_string()),
    ];
    if revalidated {
        return (StatusCode::NOT_MODIFIED, headers, Body::empty()).into_response();
    }
    (StatusCode::OK, headers, Body::from(qr.svg.clone())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Method, Request};
    use std::collections::BTreeSet;
    use tonic::codegen::Service;

    const URL: &str = "http://192.168.0.24:63309/gameplay/player?room=LANTERN";

    /// The dark modules the SVG paints, parsed back from its path, as (x, y) of the full symbol.
    fn painted_modules(svg: &str) -> BTreeSet<(usize, usize)> {
        let path = svg
            .split("<path fill=\"#000000\" d=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap();
        path.split('M')
            .filter(|square| !square.is_empty())
            .map(|square| {
                let (x, rest) = square.split_once(' ').unwrap();
                let (y, _) = rest.split_once('h').unwrap();
                (x.parse().unwrap(), y.parse().unwrap())
            })
            .collect()
    }

    #[test]
    fn the_join_link_is_the_player_page_for_the_room_at_the_base_address() {
        assert_eq!(link("http://192.168.0.24:63309", "LANTERN"), URL.to_owned());
        assert_eq!(
            link("http://127.0.0.1:63309", "ABC123"),
            "http://127.0.0.1:63309/gameplay/player?room=ABC123"
        );
    }

    #[test]
    fn the_svg_paints_exactly_the_modules_of_the_join_link_code_with_a_quiet_zone() {
        let qr = JoinQr::new(URL).unwrap();
        let code = QrCode::with_error_correction_level(URL.as_bytes(), EcLevel::M).unwrap();
        let width = code.width();
        let expected: BTreeSet<(usize, usize)> = code
            .to_colors()
            .into_iter()
            .enumerate()
            .filter(|(_, color)| *color == Color::Dark)
            .map(|(index, _)| (index % width + QUIET_ZONE, index / width + QUIET_ZONE))
            .collect();
        assert_eq!(painted_modules(&qr.svg), expected);
        let side = width + 2 * QUIET_ZONE;
        assert!(qr.svg.contains(&format!("viewBox=\"0 0 {side} {side}\"")));
        assert!(expected.iter().all(|(x, y)| *x >= QUIET_ZONE
            && *y >= QUIET_ZONE
            && *x < side - QUIET_ZONE
            && *y < side - QUIET_ZONE));
    }

    #[test]
    fn a_different_link_paints_a_different_code() {
        let other = JoinQr::new("http://192.168.0.24:63309/gameplay/player?room=ZZZZZ").unwrap();
        assert_ne!(JoinQr::new(URL).unwrap().svg, other.svg);
        assert_ne!(JoinQr::new(URL).unwrap().etag, other.etag);
    }

    #[tokio::test]
    async fn the_qr_route_serves_the_svg_with_an_entity_tag_and_revalidates_it() {
        let qr = JoinQr::new(URL).unwrap();
        let etag = qr.etag.clone();
        let mut router = router(qr);

        let served = get_qr(&mut router, HeaderMap::new()).await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.headers()[header::CONTENT_TYPE], MIME);
        assert_eq!(served.headers()[header::ETAG], etag.as_str());
        assert_eq!(served.headers()[header::CACHE_CONTROL], CACHE_CONTROL);
        let body = axum::body::to_bytes(served.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(body.starts_with(b"<svg "));

        let mut conditional = HeaderMap::new();
        conditional.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        let revalidated = get_qr(&mut router, conditional).await;
        assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED);
        let empty = axum::body::to_bytes(revalidated.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(empty.is_empty());

        let mut stale = HeaderMap::new();
        stale.insert(header::IF_NONE_MATCH, "\"stale\"".parse().unwrap());
        assert_eq!(get_qr(&mut router, stale).await.status(), StatusCode::OK);
    }

    async fn get_qr(router: &mut Router, headers: HeaderMap) -> Response {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri(QR_PATH)
            .body(Body::empty())
            .unwrap();
        *request.headers_mut() = headers;
        Service::call(router, request).await.unwrap()
    }
}
