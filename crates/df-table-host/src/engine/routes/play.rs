//! The routes of the first playable path: a TV's or a phone's `MachineView` and a seat's
//! `MachineIntent` (O.16 H3).
//!
//! They are the dev form of what a gRPC `WatchMachine` and `SubmitMachineIntent` handler would
//! call on the owner (`EngineHandle::machine_view` and `machine_intent`): the same messages, the
//! same guards, with the JSON of `machine_io` for a person and the protobuf bytes of a view
//! beside it. A view is a GET; an intent is a POST of a `WireIntentBody`, whose machine, revision
//! and operation id are filled from the seat's own view and 128 random bits when left out (a
//! counter would restart at 1 with the host and collide with the operation memory a checkpoint
//! keeps).

use axum::body::Bytes;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Query, State};
use axum::response::Response;

use crate::dev_http::{parse_body, reply};
use crate::engine::fresh_id::random_u128;
use crate::engine::machine_io::{
    WireIntentBody, WireReceiptReport, WireViewReport, receipt_report, view_report,
};
use crate::engine::report::EngineError;
use crate::engine::request::{AudienceQuery, AudienceSpec};

use super::{EngineRoutes, engine, query};

pub(super) async fn wire_view(
    State(routes): State<EngineRoutes>,
    audience_query: Result<Query<AudienceQuery>, QueryRejection>,
) -> Response {
    let result = async {
        let AudienceQuery { audience } = query(audience_query)?;
        let view = engine(&routes)?.machine_view(audience.0).await?;
        Ok::<WireViewReport, EngineError>(view_report(&view))
    }
    .await;
    reply(result)
}

pub(super) async fn wire_intent(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let intent: WireIntentBody = parse_body(&bytes)?;
        let engine = engine(&routes)?;
        let view = engine
            .machine_view(AudienceSpec::Seat { seat: intent.seat })
            .await?;
        let machine = intent.machine.clone().unwrap_or(view.machine_id);
        let observed = intent.observed.unwrap_or_else(|| {
            view.revision
                .and_then(|revision| revision.sequence)
                .unwrap_or_default()
        });
        let operation = match intent.operation {
            Some(0) => return Err(EngineError::ZeroOperation),
            Some(number) => u128::from(number),
            None => random_u128(),
        };
        let request = intent.request(&machine, observed, operation);
        let response = engine.machine_intent(intent.seat, &request).await?;
        Ok::<WireReceiptReport, EngineError>(receipt_report(&response))
    }
    .await;
    reply(result)
}
