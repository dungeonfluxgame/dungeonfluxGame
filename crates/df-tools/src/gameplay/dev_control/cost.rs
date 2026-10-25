//! `dev cost`: what this run and this session have cost, from the hub ledger's cost book, and a
//! comparison with a provider bill. Reads only; nothing here can refuse or cap a call.
use super::{Outcome, Shared, authorized, bad_request, write_unavailable};
use crate::gameplay::cost;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use std::sync::Arc;

const OFF: &str =
    "the cost book is not open: it opens with an interactive run (COST start-up line)";

/// `GET /dev/cost`: the run and session totals, by line, by cause and the top spenders.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let Some(ledger) = cost::current() else {
        return write_unavailable("cost", OFF);
    };
    Ok((StatusCode::OK, Json(ledger.costs().report(cost::now_ms()))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompareBody {
    csv: String,
}

/// `POST /dev/cost/compare {"csv": ...}`: the run's lines beside a bill's.
pub(super) async fn compare(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<CompareBody>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if body.csv.len() > cost::BILL_LIMIT_BYTES {
        return Err(bad_request("the bill is larger than 256 KiB"));
    }
    let Some(ledger) = cost::current() else {
        return write_unavailable("cost", OFF);
    };
    ledger
        .costs()
        .compare_bill(&body.csv)
        .map(|document| (StatusCode::OK, Json(document)))
        .map_err(bad_request)
}
