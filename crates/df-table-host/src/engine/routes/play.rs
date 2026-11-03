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
use axum::extract::{Query, State};
use axum::response::Response;
use serde::Deserialize;

use crate::engine::fresh_id::random_u128;
use crate::engine::machine_io::{WireIntentBody, receipt_report, view_report};
use crate::engine::report::EngineError;
use crate::engine::request::AudienceSpec;

use super::{EngineRoutes, audience_of, body, engine, reply};

#[derive(Debug, Deserialize)]
pub(super) struct ViewQuery {
    audience: Option<String>,
}

pub(super) async fn wire_view(
    State(routes): State<EngineRoutes>,
    Query(query): Query<ViewQuery>,
) -> Response {
    let result = async {
        let audience = audience_of(query.audience.as_deref())?;
        let view = engine(&routes)?.machine_view(audience).await?;
        Ok(view_report(&view))
    }
    .await;
    reply(&routes, result)
}

pub(super) async fn wire_intent(State(routes): State<EngineRoutes>, bytes: Bytes) -> Response {
    let result = async {
        let intent: WireIntentBody = body(&bytes)?;
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
            Some(number) if number > 0 => u128::from(number),
            Some(_) => {
                return Err(EngineError::BadRequest {
                    reason: "an operation id is not zero".to_owned(),
                });
            }
            None => random_u128(),
        };
        let request = intent.request(&machine, observed, operation);
        let response = engine.machine_intent(intent.seat, &request).await?;
        Ok(receipt_report(&response))
    }
    .await;
    reply(&routes, result)
}
