//! The client routes: the open pages (`GET /playground/clients.json`), a page's report
//! (`POST /dev/playground/report`) and a page's timeline
//! (`GET /playground/clients/{client}/timeline.json`).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use df_playground::media::ClientReportPost;
use df_playground::path::CLIENT_HEADER;
use df_playground::timeline::{ClientTimeline, TimelineQuery};

use super::super::files::revision;
use super::super::{done, refused, run_file_task};
use super::request::ClientsError;
use super::{check_client_name, checked_client};
use crate::dev_http::{parse_body, reply};
use crate::limits::MAXIMUM_REPORT_BYTES;
use crate::state::AppState;

/// Why a report was refused, as its audit line says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportRefusal {
    Name,
    Size,
    Body,
}

impl ReportRefusal {
    fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Body => "body",
        }
    }
}

/// The name the request's `X-Df-Client` header gives, when it has one.
fn header_client(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(CLIENT_HEADER)
        .and_then(|value| value.to_str().ok())
}

/// Records and answers a refused report.
fn refuse_report(
    state: &AppState,
    headers: &HeaderMap,
    refusal: ReportRefusal,
    status: StatusCode,
    message: String,
) -> Response {
    state.record_line(format!(
        "CLIENT report_refused client={} reason={}",
        checked_client(header_client(headers)),
        refusal.key()
    ));
    refused(status, message)
}

/// The open pages. Pages silent too long are forgotten first, each with a `CLIENT gone` line.
pub(in crate::playground) async fn clients_json(State(state): State<Arc<AppState>>) -> Response {
    let playground = state.playground();
    let now_ms = playground.clock_ms();
    for line in playground.client_table().sweep(now_ms) {
        state.record_line(line);
    }
    Json(playground.client_table().views_at(now_ms)).into_response()
}

/// A page says what it is showing and playing and what it showed since its last report. No page
/// is woken; only what changed is written to the audit log. A report is at most
/// `MAXIMUM_REPORT_BYTES`, so the whole table is bounded for each of `MAXIMUM_CLIENTS` pages.
pub(in crate::playground) async fn receive_report(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let too_large = || format!("a report is at most {MAXIMUM_REPORT_BYTES} bytes");
    let body = match body {
        Ok(body) if body.len() > MAXIMUM_REPORT_BYTES => {
            let status = StatusCode::PAYLOAD_TOO_LARGE;
            return refuse_report(&state, &headers, ReportRefusal::Size, status, too_large());
        }
        Ok(body) => body,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            let status = StatusCode::PAYLOAD_TOO_LARGE;
            return refuse_report(&state, &headers, ReportRefusal::Size, status, too_large());
        }
        Err(rejection) => {
            let status = StatusCode::BAD_REQUEST;
            let message = rejection.body_text();
            return refuse_report(&state, &headers, ReportRefusal::Body, status, message);
        }
    };
    let post: ClientReportPost = match parse_body(&body) {
        Ok(post) => post,
        Err(error) => {
            let status = StatusCode::BAD_REQUEST;
            let message = error.to_string();
            return refuse_report(&state, &headers, ReportRefusal::Body, status, message);
        }
    };
    if let Err(error) = check_client_name(&post.client) {
        let status = StatusCode::BAD_REQUEST;
        let message = error.to_string();
        return refuse_report(&state, &headers, ReportRefusal::Name, status, message);
    }
    let playground = state.playground();
    let client = post.client.clone();
    let recorded =
        playground
            .client_table()
            .record(post.client, post.report, playground.clock_ms());
    for line in recorded.lines {
        state.record_line(line);
    }
    if let Some(has) = recorded.check_revision {
        let files = playground.files().clone();
        // A revision the host cannot read is said by the files routes; this only compares.
        if let Ok(host) = run_file_task(move || revision(&files)).await
            && host != has
        {
            state.record_line(format!(
                "CLIENT stale client={client} has={} host={}",
                super::checked(&has),
                super::checked(&host)
            ));
        }
    }
    done(&state)
}

/// One page's timeline, on the playground's clock, oldest first.
pub(in crate::playground) async fn timeline_json(
    State(state): State<Arc<AppState>>,
    Path(client): Path<String>,
    query: Result<Query<TimelineQuery>, QueryRejection>,
) -> Response {
    let result = read_timeline(&state, &client, query);
    if let Err(error) = &result {
        state.record_line(format!(
            "CLIENT timeline_refused client={} error={}",
            checked_client(Some(&client)),
            error.key()
        ));
    }
    reply(result)
}

fn read_timeline(
    state: &AppState,
    client: &str,
    query: Result<Query<TimelineQuery>, QueryRejection>,
) -> Result<ClientTimeline, ClientsError> {
    let Query(query) = query.map_err(|rejection| ClientsError::InvalidQuery {
        detail: rejection.body_text(),
    })?;
    check_client_name(client).map_err(|error| ClientsError::InvalidClient {
        detail: error.to_string(),
    })?;
    let playground = state.playground();
    let now_ms = playground.clock_ms();
    for line in playground.client_table().sweep(now_ms) {
        state.record_line(line);
    }
    playground
        .client_table()
        .timeline(client, query)
        .ok_or_else(|| ClientsError::NoSuchClient {
            client: client.to_owned(),
        })
}
