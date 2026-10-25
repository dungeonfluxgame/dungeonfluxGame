//! `dev latency`: the owner's timing histograms for this server process. It reads a copy of the ledger in
//! `actor::timing_ledger` and never walks the audit ring, so the read costs the same however long the
//! game has run. It changes no game state.
//!
//! The default body is the compact summary: p50 and p95 estimates, the exact maximum and the count for
//! each metric and label. `?buckets=1` adds the cumulative fixed buckets. The export contract is in the
//! `timing_ledger` module documentation.

use super::super::actor::timing_ledger;
use super::{Outcome, Shared, authorized};
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

/// `GET /dev/latency[?buckets=1]`: the histograms and counters of every owner call and hero-line stage
/// this process has seen. Empty until the owner has reduced a call or a hero line has finished.
pub(super) async fn latency(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let buckets = params.get("buckets").is_some_and(|value| value == "1");
    let ledger = timing_ledger::snapshot();
    let mut body = ledger.export(buckets);
    body["scope"] = json!("current server process; owner call and hero-line timing");
    body["slow_call_threshold_us"] = json!(crate::gameplay::actor::SLOW_CALL_US);
    Ok((StatusCode::OK, Json(body)))
}
