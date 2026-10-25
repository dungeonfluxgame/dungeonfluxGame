//! Loopback-only development routes of the preview: the development outbox as JSON (so a tester
//! reads verification and reset links) and the fake processor's checkout page (so a tester pays a
//! test checkout and the fake webhook credits it).

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, Path, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use df_account_app::wire::{CheckoutStatus, DevSignIn};
use df_customer::DevOutbox;
use df_portal::app_api::fakes::FakePorts;
use df_types::CheckoutId;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct DevState {
    pub outbox: Arc<DevOutbox>,
    pub ports: Arc<FakePorts>,
}

pub fn router(state: DevState) -> Router {
    Router::new()
        .route("/dev/outbox", get(outbox))
        .route("/dev/checkout/{checkout_id}", get(checkout_page))
        .route("/dev/checkout/{checkout_id}/pay", post(pay))
        .route("/dev/checkout/{checkout_id}/cancel", post(cancel))
        .with_state(state)
}

/// Adds dev-only login hints to the boot response and rejects non-loopback Host headers.
pub fn with_dev_accounts(app: Router, accounts: Vec<DevSignIn>) -> Router {
    app.layer(middleware::from_fn_with_state(accounts, dev_accounts_layer))
}

async fn dev_accounts_layer(
    State(accounts): State<Vec<DevSignIn>>,
    request: axum::http::Request<Body>,
    next: Next,
) -> Response {
    let loopback_host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
        .is_some_and(|authority| matches!(authority.host(), "127.0.0.1" | "localhost" | "[::1]"));
    if !loopback_host {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "loopback_host_required"})),
        )
            .into_response();
    }
    let is_auth_boot = matches!(
        request.uri().path(),
        "/app/api/session" | "/app/api/sign-in" | "/app/api/sign-out"
    );
    let mut response = next.run(request).await;
    if !is_auth_boot || !response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, 64 * 1024).await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    value["dev_sign_in"] = json!(accounts);
    response = Response::from_parts(parts, Body::from(value.to_string()));
    response
}

fn refuse_unless_loopback(peer: SocketAddr) -> Option<Response> {
    (!peer.ip().is_loopback()).then(|| {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "loopback_only"})),
        )
            .into_response()
    })
}

fn header_value<'a>(message: &'a str, name: &str) -> &'a str {
    message
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .map_or("", str::trim)
}

async fn outbox(
    State(state): State<DevState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    if let Some(refusal) = refuse_unless_loopback(peer) {
        return refusal;
    }
    let Ok(entries) = state.outbox.index().await else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "outbox_unreadable"})),
        )
            .into_response();
    };
    let mut messages: Vec<Value> = Vec::new();
    for entry in entries {
        let Ok(Some(text)) = state.outbox.message(entry.sequence).await else {
            continue;
        };
        let links: Vec<&str> = text
            .split_whitespace()
            .filter(|word| word.starts_with("http"))
            .collect();
        messages.push(json!({
            "sequence": entry.sequence,
            "to": header_value(&text, "To:"),
            "subject": header_value(&text, "Subject:"),
            "purpose": header_value(&text, "X-DungeonFlux-Purpose:"),
            "links": links,
            "text": text,
        }));
    }
    Json(json!(messages)).into_response()
}

fn parse(checkout_id: &str) -> Option<CheckoutId> {
    CheckoutId::from_hex(checkout_id).ok()
}

async fn checkout_page(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(checkout_id): Path<String>,
) -> Response {
    if let Some(refusal) = refuse_unless_loopback(peer) {
        return refusal;
    }
    if parse(&checkout_id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // The id is validated hexadecimal, so it is safe inside markup.
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Test checkout</title>\
         <h1>Test checkout</h1><p>This stands in for the payment processor. Nothing is charged.</p>\
         <form method=\"post\" action=\"/dev/checkout/{checkout_id}/pay\">\
         <button type=\"submit\">Pay (test)</button></form>\
         <form method=\"post\" action=\"/dev/checkout/{checkout_id}/cancel\">\
         <button type=\"submit\">Cancel</button></form>"
    );
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response()
}

fn settle(
    state: &DevState,
    peer: SocketAddr,
    checkout_id: &str,
    outcome: CheckoutStatus,
) -> Response {
    if let Some(refusal) = refuse_unless_loopback(peer) {
        return refusal;
    }
    let Some(checkout) = parse(checkout_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !state.ports.checkout.deliver_webhook(checkout, outcome) {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        StatusCode::SEE_OTHER,
        [(
            header::LOCATION,
            format!("/app/credit/return?checkout={checkout_id}"),
        )],
    )
        .into_response()
}

async fn pay(
    State(state): State<DevState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(checkout_id): Path<String>,
) -> Response {
    settle(&state, peer, &checkout_id, CheckoutStatus::Paid)
}

async fn cancel(
    State(state): State<DevState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(checkout_id): Path<String>,
) -> Response {
    settle(&state, peer, &checkout_id, CheckoutStatus::Cancelled)
}
