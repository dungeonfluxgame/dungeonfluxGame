//! `dev audit` and `dev decisions`: read-only views of the shared audit path. Every audit line the
//! server prints goes through `actor::record_audit_line` or `record_audit_diagnostic`, which mirror
//! it into the bounded ring in `timeline`; these handlers read that ring and nothing else.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
};
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, parse, timeline};

/// The newest entries a read returns when no `last` is given.
const DEFAULT_LAST: &str = "50";

/// `GET /dev/audit?type=<LINE-KIND>&last=<n>`: the newest audit lines, oldest first.
pub(super) async fn audit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let last = parse::parse_last(params.get("last").map_or(DEFAULT_LAST, String::as_str))
        .map_err(bad_request)?;
    let kind = params
        .get("type")
        .map(|text| parse::parse_audit_type(text))
        .transpose()
        .map_err(bad_request)?;
    Ok((
        StatusCode::OK,
        Json(
            timeline::query(&params, "audit", last, kind.as_deref(), None, false)
                .map_err(bad_request)?,
        ),
    ))
}

/// `GET /dev/decisions?kind=<kind>&last=<n>`: the newest Jev classifications and their neutral
/// fallbacks, oldest first. `neutral` is true when the model did not decide the event.
pub(super) async fn decisions(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let last = parse::parse_last(params.get("last").map_or(DEFAULT_LAST, String::as_str))
        .map_err(bad_request)?;
    let kind = params
        .get("kind")
        .map(|text| parse::parse_decision_kind(text))
        .transpose()
        .map_err(bad_request)?;
    Ok((
        StatusCode::OK,
        Json(timeline::query(&params, "decisions", last, kind, None, false).map_err(bad_request)?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_and_decision_bounds_come_from_the_shared_parsers() {
        assert_eq!(parse::parse_last(DEFAULT_LAST), Ok(50));
        assert!(parse::parse_last("0").is_err());
        assert!(parse::parse_audit_type("SCENE-FIGHT").is_ok());
        assert!(parse::parse_decision_kind("approach").is_ok());
    }
}
