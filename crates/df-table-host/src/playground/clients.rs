//! What each open playground page last said it was showing and playing.
//!
//! A page reports every ten seconds and when its view changes. The table remembers the newest
//! report of each page by the page's own name and forgets a page that has gone quiet, so
//! `clients` shows the pages that are open. A report is a reading, not a change: it is not an
//! audit line and wakes nobody.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use df_playground::media::{ClientReport, ClientReportPost};

use super::{done, refused, reject_body};
use crate::dev_http::parse_body;
use crate::limits::{
    FORGET_CLIENT_AFTER, MAXIMUM_CLIENT_NAME, MAXIMUM_CLIENTS, MAXIMUM_REPORT_BYTES,
};
use crate::state::AppState;
use crate::wire::ClientView;

/// Why a page's name is not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientNameError {
    Empty,
    TooLong,
    BadCharacter(char),
}

impl fmt::Display for ClientNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a client name is letters, digits, '_' and '-', up to {MAXIMUM_CLIENT_NAME} characters"
        )?;
        match self {
            Self::Empty => formatter.write_str(" (it is empty)"),
            Self::TooLong => formatter.write_str(" (it is longer)"),
            Self::BadCharacter(character) => write!(formatter, " ('{character}' is not allowed)"),
        }
    }
}

/// Checks the name a page gives itself.
pub fn check_client_name(name: &str) -> Result<(), ClientNameError> {
    if name.is_empty() {
        return Err(ClientNameError::Empty);
    }
    if name.len() > MAXIMUM_CLIENT_NAME {
        return Err(ClientNameError::TooLong);
    }
    match name
        .chars()
        .find(|character| !(character.is_ascii_alphanumeric() || "_-".contains(*character)))
    {
        Some(character) => Err(ClientNameError::BadCharacter(character)),
        None => Ok(()),
    }
}

/// Each page's newest report with the time it came, in milliseconds on the playground's clock.
type Heard = BTreeMap<String, (u64, ClientReport)>;

/// The pages heard from, by name.
#[derive(Debug, Default)]
pub struct ClientTable {
    heard: Mutex<Heard>,
}

impl ClientTable {
    fn lock(&self) -> MutexGuard<'_, Heard> {
        self.heard.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers what a page said at `now_ms`. The report is bounded, and past `MAXIMUM_CLIENTS` a
    /// new page replaces the page heard from longest ago.
    pub fn record(&self, client: String, report: ClientReport, now_ms: u64) {
        let mut heard = self.lock();
        if !heard.contains_key(&client)
            && heard.len() >= MAXIMUM_CLIENTS
            && let Some(oldest) = heard
                .iter()
                .min_by_key(|(_, (when, _))| *when)
                .map(|(name, _)| name.clone())
        {
            heard.remove(&oldest);
        }
        heard.insert(client, (now_ms, report.bounded()));
    }

    /// The pages heard from as of `now_ms`, with how long ago each last spoke. A page silent for
    /// `FORGET_CLIENT_AFTER` is dropped from the answer and from memory.
    pub fn views_at(&self, now_ms: u64) -> Vec<ClientView> {
        let forget_after_ms = u64::try_from(FORGET_CLIENT_AFTER.as_millis()).unwrap_or(u64::MAX);
        let mut heard = self.lock();
        heard.retain(|_, (when_ms, _)| now_ms.saturating_sub(*when_ms) < forget_after_ms);
        heard
            .iter()
            .map(|(client, (when_ms, report))| ClientView {
                client: client.clone(),
                age_ms: now_ms.saturating_sub(*when_ms),
                report: report.clone(),
            })
            .collect()
    }
}

pub(super) async fn clients_json(State(state): State<Arc<AppState>>) -> Response {
    Json(state.playground().clients()).into_response()
}

/// A page says what it is showing and playing. Nothing is recorded in the audit log and no other
/// page is woken: a report is a reading, not a change. A report is at most `MAXIMUM_REPORT_BYTES`,
/// so the whole table is at most that many bytes for each of `MAXIMUM_CLIENTS` pages.
pub(super) async fn receive_report(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    if body.len() > MAXIMUM_REPORT_BYTES {
        return refused(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("a report is at most {MAXIMUM_REPORT_BYTES} bytes"),
        );
    }
    let post: ClientReportPost = match parse_body(&body) {
        Ok(post) => post,
        Err(error) => return reject_body(&state, &error),
    };
    if let Err(error) = check_client_name(&post.client) {
        return refused(StatusCode::BAD_REQUEST, error.to_string());
    }
    state.playground().record_client(post.client, post.report);
    done(&state)
}
