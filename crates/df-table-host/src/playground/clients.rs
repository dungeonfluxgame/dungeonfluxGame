//! What each open playground page last said it was showing and playing.
//!
//! A page reports every ten seconds and when its view changes. The table remembers the newest
//! report of each page by the page's own name and forgets a page that has gone quiet, so
//! `clients` shows the pages that are open. A report is a reading, not a change: it is not an
//! audit line and wakes nobody.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use df_playground::media::{ClientReport, ClientReportPost};

use super::{done, refused, reject_malformed};
use crate::limits::{FORGET_CLIENT_AFTER, MAXIMUM_CLIENT_NAME, MAXIMUM_CLIENTS};
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

type Heard = BTreeMap<String, (Instant, ClientReport)>;

/// The pages heard from, by name.
#[derive(Debug, Default)]
pub struct ClientTable {
    heard: Mutex<Heard>,
}

impl ClientTable {
    fn lock(&self) -> MutexGuard<'_, Heard> {
        self.heard.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers what a page said. The report is bounded, and past `MAXIMUM_CLIENTS` a new page
    /// replaces the page heard from longest ago.
    pub fn record(&self, client: String, report: ClientReport) {
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
        heard.insert(client, (Instant::now(), report.bounded()));
    }

    /// The pages heard from as of `now`, with how long ago each last spoke. A page silent for
    /// `FORGET_CLIENT_AFTER` is dropped from the answer and from memory.
    pub fn views_at(&self, now: Instant) -> Vec<ClientView> {
        let mut heard = self.lock();
        heard.retain(|_, (when, _)| now.saturating_duration_since(*when) < FORGET_CLIENT_AFTER);
        heard
            .iter()
            .map(|(client, (when, report))| ClientView {
                client: client.clone(),
                age_ms: u64::try_from(now.saturating_duration_since(*when).as_millis())
                    .unwrap_or(u64::MAX),
                report: report.clone(),
            })
            .collect()
    }
}

pub(super) async fn clients_json(State(state): State<Arc<AppState>>) -> Response {
    Json(state.playground().clients()).into_response()
}

/// A page says what it is showing and playing. Nothing is recorded in the audit log and no other
/// page is woken: a report is a reading, not a change.
pub(super) async fn receive_report(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let post: ClientReportPost = match serde_json::from_slice(&body) {
        Ok(post) => post,
        Err(error) => return reject_malformed(&state, &error),
    };
    if let Err(error) = check_client_name(&post.client) {
        return refused(StatusCode::BAD_REQUEST, error.to_string());
    }
    state.playground().record_client(post.client, post.report);
    done(&state)
}
