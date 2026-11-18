//! The client routes: the open pages (`GET /playground/clients.json`), a page's report
//! (`POST /dev/playground/report`) and a page's timeline
//! (`GET /playground/clients/{client}/timeline.json`).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use df_playground::media::ClientReportPost;
use df_playground::path::CLIENT_HEADER;
use df_playground::timeline::{ClientTimeline, TimelineQuery};

use super::super::FileError;
use super::super::files::revision;
use super::request::ClientsError;
use super::{check_client_name, checked_client};
use crate::blocking::run_blocking;
use crate::dev_http::{BodyCause, BodyError, read_body, read_query, reply};
use crate::state::AppState;
use df_playground::limits::MAXIMUM_REPORT_BYTES;

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

/// Records a refused report and gives its refusal.
fn refuse_report(
    state: &AppState,
    headers: &HeaderMap,
    refusal: ReportRefusal,
    error: ClientsError,
) -> ClientsError {
    state.record_line(format!(
        "CLIENT report_refused client={} reason={}",
        checked_client(header_client(headers)),
        refusal.key()
    ));
    error
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
/// `MAXIMUM_REPORT_BYTES`, so the whole table is bounded for each of `MAXIMUM_CLIENTS` pages. The
/// player listener says where a report came from (`ConnectInfo`); the dev listener does not.
pub(in crate::playground) async fn receive_report(
    State(state): State<Arc<AppState>>,
    connection: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let remote = connection.map(|Extension(ConnectInfo(address))| address.ip());
    let result = read_report(body).and_then(|post| {
        check_client_name(&post.client)
            .map(|()| post)
            .map_err(|error| ClientsError::InvalidClient {
                detail: error.to_string(),
            })
    });
    let post = match result {
        Ok(post) => post,
        Err(error) => {
            let refusal = match &error {
                ClientsError::InvalidBody(body) if body.cause == BodyCause::TooLarge => {
                    ReportRefusal::Size
                }
                ClientsError::InvalidClient { .. } => ReportRefusal::Name,
                _ => ReportRefusal::Body,
            };
            return reply::<(), _>(Err(refuse_report(&state, &headers, refusal, error)));
        }
    };
    let playground = state.playground();
    let client = post.client.clone();
    let recorded =
        playground
            .client_table()
            .record(post.client, post.report, remote, playground.clock_ms());
    for line in recorded.lines {
        state.record_line(line);
    }
    if let Some(has) = recorded.check_revision {
        let files = playground.files().clone();
        // A revision the host cannot read is said by the files routes; this only compares.
        if let Ok(host) = run_blocking(move || revision(&files), FileError::Stopped).await
            && host != has
        {
            state.record_line(format!(
                "CLIENT stale client={client} has={} host={}",
                super::checked(&has),
                super::checked(&host)
            ));
        }
    }
    reply::<_, ClientsError>(Ok(()))
}

/// The report in a body: at most `MAXIMUM_REPORT_BYTES`, and the JSON of a report.
fn read_report(body: Result<Bytes, BytesRejection>) -> Result<ClientReportPost, ClientsError> {
    match body {
        Ok(body) if body.len() > MAXIMUM_REPORT_BYTES => Err(ClientsError::InvalidBody(
            BodyError::too_large(MAXIMUM_REPORT_BYTES),
        )),
        body => Ok(read_body(body)?),
    }
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
    let query = read_query(query)?;
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
