//! The routes of the first playable path: a TV's or a phone's `MachineView` and a seat's
//! `MachineIntent` (O.16 H3).
//!
//! They are the dev form of what a gRPC `WatchMachine` and `SubmitMachineIntent` handler would
//! call on the owner (`EngineHandle::machine_view` and `machine_intent`): the same messages, the
//! same guards, with the JSON of `machine_io` for a person and the protobuf bytes of a view
//! beside it. A view is a GET; an intent is a POST of a `WireIntentBody`, whose machine, revision
//! and operation id are filled from the seat's own view and 128 random bits when left out (a
//! counter would restart at 1 with the host and collide with the operation memory a checkpoint
//! keeps). The view read to fill an intent writes no line of its own; an intent refused before
//! it reaches the owner (its seat's view could not be read, its operation id is zero) still
//! writes its `wire_intent` line.

use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, QueryRejection};
use axum::extract::{Query, State};
use axum::response::Response;
use df_protocol::common as wire;
use tokio::time::Instant;

use crate::engine::audit::WireIntentOutcome;
use crate::engine::fresh_id::random_u128;
use crate::engine::handle::EngineHandle;
use crate::engine::machine_io::{
    WireIntentBody, WireReceiptReport, WireViewReport, receipt_report, view_report,
};
use crate::engine::report::EngineError;
use crate::engine::request::{AudienceQuery, AudienceSpec};

use super::{EngineRoutes, answer, body, engine, query};

pub(super) async fn wire_view(
    State(routes): State<EngineRoutes>,
    audience_query: Result<Query<AudienceQuery>, QueryRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let AudienceQuery { audience } = query(audience_query)?;
        let view = engine(&routes)?.machine_view(audience.0).await?;
        Ok::<WireViewReport, EngineError>(view_report(&view))
    }
    .await;
    answer(&routes, "wire_view", started, result)
}

/// The request a body stands for, with what it left out filled from the seat's own view.
async fn filled_request(
    engine: &EngineHandle,
    intent: &WireIntentBody,
) -> Result<wire::SubmitMachineIntentRequest, EngineError> {
    let view = engine
        .intent_view(AudienceSpec::Seat { seat: intent.seat })
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
    Ok(intent.request(&machine, observed, operation))
}

pub(super) async fn wire_intent(
    State(routes): State<EngineRoutes>,
    bytes: Result<Bytes, BytesRejection>,
) -> Response {
    let started = Instant::now();
    let result = async {
        let intent: WireIntentBody = body(bytes)?;
        let engine = engine(&routes)?;
        let request = match filled_request(engine, &intent).await {
            Ok(request) => request,
            Err(error) => {
                engine.record_wire_intent(
                    intent.seat,
                    None,
                    WireIntentOutcome::Refused,
                    Some(error.key()),
                );
                return Err(error);
            }
        };
        let response = engine.machine_intent(intent.seat, &request).await?;
        Ok::<WireReceiptReport, EngineError>(receipt_report(&response))
    }
    .await;
    answer(&routes, "wire_intent", started, result)
}
