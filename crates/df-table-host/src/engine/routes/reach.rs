//! The routes that reach into the owner for reads, and the one change of the event level:
//! `inspect <report>` and `events level`.
//!
//! An inspect report is a GET of `/dev/engine/inspect/{report}` for one audience (the operator
//! when none is named). The event level is the level of the host's sinks, the app's (the canvas's
//! STEP lines) and the engine's (its PROJECTION and other class B lines): a GET reads it and a
//! POST sets both, which writes `HOST sink_level from=<k> to=<k> driven=true`. Class A is recorded
//! at either level. The level is an atomic of each sink, so it changes without the owner thread
//! and also while the engine is not running.

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use df_table::inspect::InspectAudience;
use tokio::time::Instant;

use crate::engine::report::EngineError;
use crate::engine::report::reach::EventLevelReport;
use crate::engine::request::{EventLevelRequest, InspectName, InspectQuery};
use crate::observe::EventLevel;

use super::{EngineRoutes, answer, body, engine, query};

pub(super) async fn inspect(
    State(routes): State<EngineRoutes>,
    Path(report): Path<String>,
    inspect_query: Result<Query<InspectQuery>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let name = InspectName::from_key(&report).ok_or(EngineError::UnknownReport { report })?;
        let InspectQuery { audience } = query(inspect_query)?;
        let audience = audience.map_or(InspectAudience::Operator, |audience| audience.0);
        engine(&routes)?.inspect(name, audience).await
    }
    .await;
    answer(&routes, "inspect", started, result)
}

/// The level in force: the app's sink, which the engine's follows.
fn level_now(routes: &EngineRoutes) -> EventLevel {
    routes.app.sink().level()
}

pub(super) async fn events_level(State(routes): State<EngineRoutes>) -> Response {
    let started = Instant::now();
    let result = Ok::<EventLevelReport, EngineError>(EventLevelReport {
        level: level_now(&routes),
    });
    answer(&routes, "events_level", started, result)
}

pub(super) async fn events_level_set(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = body::<EventLevelRequest>(bytes).map(|EventLevelRequest { level }| {
        let from = level_now(&routes);
        routes.app.sink().set_level(level);
        if let Ok(engine) = engine(&routes) {
            engine.set_event_level(level);
        }
        routes.app.record_line(format!(
            "HOST sink_level from={} to={} driven=true",
            from.key(),
            level.key()
        ));
        EventLevelReport { level }
    });
    answer(&routes, "events_level", started, result)
}
