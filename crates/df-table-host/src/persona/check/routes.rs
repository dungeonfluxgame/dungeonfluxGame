//! The check routes: `POST /dev/persona/check/judge` (the judge's typed answer) and
//! `POST /dev/persona/check/run` (the judge, the call, the roll, the card and the result).
//!
//! Each answers a `DevReply` of its value or a typed [`CheckRouteError`] with one
//! `PERSONA refused verb=check error=<key>` audit line. A run is a paid sequence (model calls and
//! the table's time), so it runs on a task of its own that the handler waits for. The dice are the
//! host's recording dice over system entropy (`RecordedDice::from_system_entropy`): a run is never
//! seeded in the host, and a test passes its own entropy.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;

use super::flow::{RunClock, run_check};
use super::judge::judge_check;
use super::limits::{LINE_BUDGET_MS, MAXIMUM_REMEMBERED_FLOURISHES, ROLL_WATCH_MS};
use super::pending::PendingRolls;
use super::request::{CheckJudgeRequest, CheckRouteError, CheckRunRequest, CheckTest};
use crate::adapters::{ActionBook, DfRulesOwner, RecordedDice, RulesCheckKit, SeatRoster};
use crate::blocking::run_owned;
use crate::dev_http::{parse_body, reply};
use crate::persona::routes::PersonaRoutes;
use crate::state::AppState;

/// The addresses of the check routes.
pub(in crate::persona) mod path {
    /// Asks the check judge whether a roll is called (POST a `CheckJudgeRequest`).
    pub const PERSONA_CHECK_JUDGE: &str = "/dev/persona/check/judge";
    /// Judges, calls, rolls, shows and reads one check (POST a `CheckRunRequest`).
    pub const PERSONA_CHECK_RUN: &str = "/dev/persona/check/run";
}

/// What the check routes work with: the persona routes (the host, the text surface, the speech
/// service and the dungeon master's box) and the kit that draws and resolves a roll.
pub(crate) struct CheckRoutes {
    pub(super) persona: Arc<PersonaRoutes>,
    pub(super) kit: RulesCheckKit,
    /// How long a line of the dungeon master may take before it is said as its numbers alone.
    pub(super) line_budget: Duration,
    /// The last flourishes the dungeon master said, oldest first, so the next does not repeat them.
    said: Mutex<VecDeque<String>>,
    /// The table's turn: one check runs at a time (its card, its lines and its clearing), whoever
    /// asked for it, the runner or a seat.
    table: tokio::sync::Mutex<()>,
    /// The tests each character rolled from a seat at each place, by (character, place): the
    /// judge is told them, so a check is not rolled again until something changes.
    seat_checked: Mutex<SeatChecked>,
    /// The roll a seated hero's player is asked to make now.
    pub(super) pending: PendingRolls,
    /// How long a called roll waits for the player before the host rolls it.
    pub(super) roll_watch: Duration,
}

/// The tests rolled from a seat, by (character, place).
type SeatChecked = BTreeMap<(String, Option<String>), Vec<CheckTest>>;

impl CheckRoutes {
    pub(super) fn new(
        persona: Arc<PersonaRoutes>,
        kit: RulesCheckKit,
        line_budget: Duration,
    ) -> Self {
        Self {
            persona,
            kit,
            line_budget,
            said: Mutex::new(VecDeque::new()),
            table: tokio::sync::Mutex::new(()),
            seat_checked: Mutex::new(BTreeMap::new()),
            pending: PendingRolls::default(),
            roll_watch: Duration::from_millis(ROLL_WATCH_MS),
        }
    }

    /// Waits for the table's turn; the check runs while the guard is held.
    pub(super) async fn table_turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.table.lock().await
    }

    /// The tests `character` rolled from a seat at `place`.
    pub(super) fn seat_checked(&self, character: &str, place: Option<&str>) -> Vec<CheckTest> {
        self.seat_checked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(character.to_owned(), place.map(str::to_owned)))
            .cloned()
            .unwrap_or_default()
    }

    /// Keeps `test` as rolled from a seat by `character` at `place`.
    pub(super) fn keep_seat_checked(&self, character: &str, place: Option<&str>, test: CheckTest) {
        self.seat_checked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry((character.to_owned(), place.map(str::to_owned)))
            .or_default()
            .push(test);
    }

    /// The flourishes said last, oldest first.
    pub(super) fn said_before(&self) -> Vec<String> {
        self.said
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Keeps `flourish` among the last said; the oldest is forgotten past
    /// `MAXIMUM_REMEMBERED_FLOURISHES`. A line said without a flourish is not kept.
    pub(super) fn remember(&self, flourish: String) {
        if flourish.is_empty() {
            return;
        }
        let mut said = self.said.lock().unwrap_or_else(PoisonError::into_inner);
        said.push_back(flourish);
        while said.len() > MAXIMUM_REMEMBERED_FLOURISHES {
            said.pop_front();
        }
    }
}

fn refused<T>(app: &AppState, result: &Result<T, CheckRouteError>) {
    if let Err(error) = result {
        app.record_line(format!("PERSONA refused verb=check error={}", error.key()));
    }
}

async fn post_judge(State(routes): State<Arc<CheckRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<CheckJudgeRequest>(&bytes) {
        Ok(request) => {
            let owned = Arc::clone(&routes);
            run_owned(
                async move { judge_check(&owned.persona.app, &owned.persona.text, &request).await },
                CheckRouteError::Stopped,
            )
            .await
        }
        Err(error) => Err(CheckRouteError::InvalidBody(error)),
    };
    refused(&routes.persona.app, &result);
    reply(result)
}

async fn post_run(State(routes): State<Arc<CheckRoutes>>, bytes: Bytes) -> Response {
    let result = match parse_body::<CheckRunRequest>(&bytes) {
        Ok(request) => {
            let owned = Arc::clone(&routes);
            run_owned(
                async move {
                    let _turn = owned.table_turn().await;
                    run_check(&owned, &RunClock::start(), &request).await
                },
                CheckRouteError::Stopped,
            )
            .await
        }
        Err(error) => Err(CheckRouteError::InvalidBody(error)),
    };
    refused(&routes.persona.app, &result);
    reply(result)
}

/// The check routes over the persona routes, rolling on `dice`.
pub(in crate::persona) fn router_over(
    persona: Arc<PersonaRoutes>,
    dice: Arc<RecordedDice>,
) -> Router {
    let rules = Arc::new(DfRulesOwner::new(
        SeatRoster::default(),
        ActionBook::default(),
    ));
    let app = Arc::clone(&persona.app);
    let routes = Arc::new(CheckRoutes::new(
        persona,
        RulesCheckKit::new(rules, dice),
        Duration::from_millis(LINE_BUDGET_MS),
    ));
    // A seat's roll reaches the same runner from either listener (`seat_roll`).
    app.set_check_rolls(Arc::clone(&routes));
    Router::new()
        .route(path::PERSONA_CHECK_JUDGE, post(post_judge))
        .route(path::PERSONA_CHECK_RUN, post(post_run))
        .with_state(routes)
}
