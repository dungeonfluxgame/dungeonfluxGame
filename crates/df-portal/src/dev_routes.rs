//! `dev accounts` and `dev outbox` (coding style 8.8): read-only JSON views for the loopback,
//! token-guarded development surface. Addresses are masked; an outbox message is shown in full
//! because reading its link is the point of the development outbox.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use df_customer::{AccountStore, DevOutbox, EmailSender};
use serde_json::{Value, json};

use crate::{PortalState, html::hex};

struct DevState<S, M> {
    portal: PortalState<S, M>,
    outbox: Option<Arc<DevOutbox>>,
}

impl<S, M> Clone for DevState<S, M> {
    fn clone(&self) -> Self {
        Self {
            portal: self.portal.clone(),
            outbox: self.outbox.clone(),
        }
    }
}

pub fn router<S: AccountStore, M: EmailSender>(
    portal: PortalState<S, M>,
    outbox: Option<Arc<DevOutbox>>,
) -> Router {
    Router::new()
        .route("/dev/accounts", get(accounts::<S, M>))
        .route("/dev/outbox", get(outbox_index::<S, M>))
        .route("/dev/outbox/{sequence}", get(outbox_message::<S, M>))
        .with_state(DevState { portal, outbox })
}

fn unavailable(reason: &'static str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": reason })),
    )
        .into_response()
}

async fn accounts<S: AccountStore, M: EmailSender>(
    State(state): State<DevState<S, M>>,
) -> Response {
    let summary = match state.portal.service.dev_summary().await {
        Ok(summary) => summary,
        Err(error) => return unavailable(error.label()),
    };
    let recent: Vec<Value> = summary
        .recent
        .iter()
        .map(|account| {
            json!({
                "account": hex(account.id.as_bytes()),
                "email": account.email.as_ref().map(|email| email.masked()),
                "status": account.status.label(),
                "verified": account.is_verified,
                "active_sessions": account.active_sessions,
                "last_event": account.last_event.map(|event| json!({
                    "event": event.kind.label(),
                    "at_ms": event.at_ms,
                })),
            })
        })
        .collect();
    Json(json!({
        "active": summary.active,
        "closing": summary.closing,
        "closed": summary.closed,
        "verified": summary.verified,
        "recent": recent,
    }))
    .into_response()
}

async fn outbox_index<S, M>(State(state): State<DevState<S, M>>) -> Response {
    let Some(outbox) = state.outbox else {
        return unavailable("no_dev_outbox");
    };
    match outbox.index().await {
        Ok(entries) => Json(json!({
            "directory": outbox.directory().display().to_string(),
            "messages": entries
                .iter()
                .map(|entry| json!({ "sequence": entry.sequence, "file": entry.file_name }))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(_) => unavailable("outbox_unreadable"),
    }
}

async fn outbox_message<S, M>(
    State(state): State<DevState<S, M>>,
    Path(sequence): Path<u64>,
) -> Response {
    let Some(outbox) = state.outbox else {
        return unavailable("no_dev_outbox");
    };
    match outbox.message(sequence).await {
        Ok(Some(message)) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            message,
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no_such_message" })),
        )
            .into_response(),
        Err(_) => unavailable("outbox_unreadable"),
    }
}
