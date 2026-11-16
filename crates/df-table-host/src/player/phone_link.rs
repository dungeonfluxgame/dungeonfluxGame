//! The phone link page of the loopback dev listener: the phone page's address in large text and as
//! a QR code, so the owner can scan it from the PC's screen, and the friends waiting for the host's
//! approval with the commands that answer them. Without a player listener it says how to start
//! one. The QR code is drawn as SVG from the code's module matrix; nothing here is a
//! caller's text (the links are made by `player` from an address and a port).

use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::State;
use axum::response::Html;
use qrcode::{Color, QrCode};

use crate::render::escape;
use crate::seat::request::{PendingLine, PendingState};
use crate::state::AppState;

/// The light modules around a QR code that a camera needs to find it.
const QUIET_MODULES: usize = 4;
/// How large the code is drawn on the PC's screen, in CSS pixels.
const QR_DRAWN_PX: usize = 360;

/// The SVG of a QR code of `text`, or why it could not be made.
pub(super) fn qr_svg(text: &str) -> Result<String, qrcode::types::QrError> {
    let code = QrCode::new(text.as_bytes())?;
    let width = code.width();
    let size = width + 2 * QUIET_MODULES;
    let mut dark = String::new();
    for (index, color) in code.to_colors().into_iter().enumerate() {
        if color == Color::Dark {
            let (x, y) = (index % width + QUIET_MODULES, index / width + QUIET_MODULES);
            // Writing to a `String` cannot fail.
            let _ = write!(dark, "M{x} {y}h1v1h-1z");
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" \
         width=\"{QR_DRAWN_PX}\" height=\"{QR_DRAWN_PX}\" shape-rendering=\"crispEdges\" \
         role=\"img\" aria-label=\"QR code of the phone page\">\
         <rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/>\
         <path d=\"{dark}\" fill=\"#000\"/></svg>"
    ))
}

/// The page's body for `links`: each address with its code, or how to start the listener.
pub(super) fn body(links: &[String]) -> String {
    if links.is_empty() {
        return "<h1>Phone page</h1>\
                <p>The player listener is off. Start the host with it, for example:</p>\
                <pre>df-table serve --port 7879 --players 7880</pre>\
                <p>then open this page again.</p>"
            .to_owned();
    }
    let mut body = "<h1>Phone page</h1>\
                    <p>Scan with the phone's camera, on the same Wi-Fi as this PC.</p>"
        .to_owned();
    for link in links {
        let code = qr_svg(link)
            .unwrap_or_else(|error| format!("<p>No QR code for this address: {error}.</p>"));
        // Writing to a `String` cannot fail.
        let _ = write!(body, "<p class=\"link\">{link}</p>{code}");
    }
    body
}

/// The friends' requests waiting for the host, with the commands that answer them. The page has
/// no script, so the answer is given on the command line.
pub(super) fn pending_body(pending: &[PendingLine]) -> String {
    let waiting: Vec<&PendingLine> = pending
        .iter()
        .filter(|line| line.state == PendingState::Waiting)
        .collect();
    if waiting.is_empty() {
        return "<h2>Waiting for approval</h2><p>Nobody is waiting.</p>".to_owned();
    }
    let mut body = "<h2>Waiting for approval</h2><ul>".to_owned();
    for line in waiting {
        // Writing to a `String` cannot fail.
        let _ = write!(
            body,
            "<li>{} (friend {}): <code>df-table players approve {}</code> or \
             <code>df-table players refuse {}</code></li>",
            escape(&line.name),
            line.profile,
            line.request,
            line.request
        );
    }
    body.push_str("</ul>");
    body
}

/// `GET path::PHONE_LINK`. It reloads itself every few seconds, so a new request shows up.
pub(super) async fn page(State(state): State<Arc<AppState>>) -> Html<String> {
    let pending = match state.seats().open().await {
        Ok(mut open) => pending_body(&open.books().report().pending),
        Err(error) => format!(
            "<p>The players books cannot be read: {}.</p>",
            escape(&error.to_string())
        ),
    };
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"5\">\
         <title>Phone page</title><style>\
         body {{ margin: 0; padding: 24px; background: #09131d; color: #f2e8d5; \
         font: 18px/1.5 system-ui, sans-serif; }}\
         .link {{ font-size: 40px; font-weight: 600; word-break: break-all; }}\
         svg {{ display: block; margin: 16px 0 32px; }}\
         </style></head><body>{}{pending}</body></html>",
        body(state.player_links())
    ))
}
