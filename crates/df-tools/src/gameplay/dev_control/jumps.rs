//! Development jumps: checkpoint restore, back/forward history and state
//! overrides (milestones 2/3).
//!
//! Every jump is reduced on the single owner thread through the same
//! compare-and-swap, receipt and publication path as player decisions, so the
//! database, the in-memory checkpoint and every connected screen stay
//! consistent. The candidate is built there from the owner's own current
//! checkpoint: nothing here writes game state directly, and a jump can never
//! produce a state that fails the owner's own candidate gate.
//!
//! History rules: stepping back moves the stepped-over states to the redo
//! stack; stepping forward consumes it; restore/override replace it with the
//! pre-jump state (every jump is itself undoable once). Any other commit
//! forks history: the redo stack is dropped and the next dev command reports
//! how many entries went. Provider calls are never repeated by jumps: dev
//! candidates stage no intents, effects, facts or draws, so no text, voice or
//! pictures are generated or regenerated on replay; cached artifacts are
//! keyed by content and reused by the views.

use axum::{extract::State, http::HeaderMap, response::Json};
use serde_json::{Value, json};
use std::sync::Arc;

use super::{Outcome, Shared, authorized, bad_request, display_view, failed};
use crate::gameplay::dev_control::{commands, history};

/// The current revision as a plain pair, read from the display view.
async fn current_revision(
    state: &Shared,
) -> Result<
    (history::Revision, df_protocol::common::SessionRevision),
    (axum::http::StatusCode, Json<Value>),
> {
    let (revision, _) = display_view(state).await?;
    let epoch = revision
        .epoch
        .as_ref()
        .and_then(|epoch| epoch.value)
        .unwrap_or(0);
    Ok(((epoch, revision.sequence.unwrap_or(0)), revision))
}

/// Drops a redo stack forked by playing on, reporting what went.
fn take_stale_notices(state: &Shared, current: history::Revision) -> Vec<String> {
    let Ok(mut guarded) = state.history.lock() else {
        return vec!["history lock poisoned".to_owned()];
    };
    let before = guarded.redo_len();
    guarded.drop_stale_redo(current);
    if guarded.redo_len() == 0 && before > 0 {
        vec![format!(
            "dropped {before} redo entries: played on after stepping back, history forked"
        )]
    } else {
        Vec::new()
    }
}

/// Sends one dev order to the owner and maps its answer without rewording.
async fn owner_jump(
    state: &Shared,
    credential: [u8; 32],
    job: commands::DevJob,
) -> Result<(history::Revision, String, String), (axum::http::StatusCode, Json<Value>)> {
    let inventory = {
        let recovery = &state.recovery;
        commands::OwnedInventory {
            rules: recovery.rules.clone(),
            content: recovery.content.clone(),
            resources: recovery.resources.clone(),
            assets: recovery.assets.clone(),
            limits: recovery.limits,
        }
    };
    let order = commands::DevOrder { job, inventory };
    let (reply, wait) = tokio::sync::oneshot::channel();
    state
        .service
        .actor
        .try_submit(super::super::actor::Call::Dev {
            credential,
            order: Box::new(order),
            reply,
        })
        .map_err(|_| failed("gameplay actor queue full"))?;
    let result = tokio::time::timeout(super::super::timing::active().reply, wait)
        .await
        .map_err(|_| failed("gameplay dev deadline"))?
        .map_err(|_| failed("gameplay actor stopped"))?;
    match result {
        Ok(answer) => Ok(((answer.epoch, answer.sequence), answer.before, answer.after)),
        Err(status) => Err(map_dev_status(status)),
    }
}

fn map_dev_status(status: tonic::Status) -> (axum::http::StatusCode, Json<Value>) {
    use tonic::Code;
    match status.code() {
        Code::FailedPrecondition => {
            let message = status.message().to_owned();
            if message.contains("candidate refused") {
                (
                    axum::http::StatusCode::OK,
                    Json(
                        json!({"status": "refused", "code": "invalid-candidate", "reason": message}),
                    ),
                )
            } else {
                (
                    axum::http::StatusCode::OK,
                    Json(json!({"status": "observation", "reason": message})),
                )
            }
        }
        _ => failed(status.message()),
    }
}

/// The first joined seat's credential scopes table-level jumps.
fn table_credential(
    seats: &super::Seats,
) -> Result<[u8; 32], (axum::http::StatusCode, Json<Value>)> {
    seats
        .seats
        .iter()
        .find_map(|seat| seat.credential)
        .ok_or_else(|| bad_request("no seat has joined yet"))
}

/// Rewinds and overrides share one kill switch. Restores and back/forward now
/// run through the owner-supported rewind (`commit_owner_rewind` with full
/// saved moments); overrides still need their projection-safety proof, so they
/// stay refused until milestone 3 lands it.
fn jumps_disabled() -> Outcome {
    Err((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "unavailable",
            "reason": "override is disabled: direct edits need the milestone 3 projection-safety proof; restore/back/forward are live",
        })),
    ))
}

/// Kill switch for rewinds: a plain function (not a const) so the gated
/// implementations below stay compiled and tested while refusing live
/// traffic. Rewinds are ON: they commit the saved moment's full state through
/// the owner-supported rewind, which the view projection accepts.
fn jumps_off() -> bool {
    false
}

/// Kill switch for overrides (milestone 3): still off.
fn overrides_off() -> bool {
    true
}

/// `POST /dev/restore {"name": "<snapshot>"}`: returns to a named point.
/// The pre-jump state becomes the new single redo entry.
pub(super) async fn restore_named(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if jumps_off() {
        return jumps_disabled();
    }
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("restore needs {\"name\": \"<snapshot>\"}"))?;
    let moment = state
        .history
        .lock()
        .map_err(|_| failed("history lock poisoned"))?
        .named(name)
        .ok_or_else(|| bad_request(format!("no checkpoint named {name:?}")))?;
    let (current, _) = current_revision(&state).await?;
    let mut notices = take_stale_notices(&state, current);
    let credential = {
        let seats = state.seats.lock().await;
        table_credential(&seats)?
    };
    let pre = pre_jump_moment(&state);
    let what = format!("checkpoint {name:?}");
    let (revision, before, after) = owner_jump(
        &state,
        credential,
        commands::DevJob::Restore {
            moment: Box::new(moment),
            what: what.clone(),
        },
    )
    .await?;
    if let Ok(mut guarded) = state.history.lock() {
        guarded.set_redo(pre, revision);
    }
    notices.push(format!("restored {what}"));
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "restored",
            "name": name,
            "revision": {"epoch": revision.0, "sequence": revision.1},
            "before": before,
            "after": after,
            "notices": notices,
        })),
    ))
}

/// The owner's current moment, for the pre-jump redo entry: the past
/// ring's newest entry, since the publication hook records every commit.
fn pre_jump_moment(state: &Shared) -> Vec<history::SavedMoment> {
    state
        .history
        .lock()
        .ok()
        .and_then(|guarded| guarded.latest())
        .into_iter()
        .collect()
}

/// `POST /dev/back {"n": 2}`: returns to the state before the last n committed
/// decisions. The stepped-over states become the redo stack.
pub(super) async fn step_back(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if jumps_off() {
        return jumps_disabled();
    }
    let n = body.get("n").and_then(Value::as_u64).unwrap_or(1);
    if n == 0 || n > 64 {
        return Err(bad_request("back needs {\"n\": 1..=64}"));
    }
    let (current, _) = current_revision(&state).await?;
    let mut notices = take_stale_notices(&state, current);
    let (moment, stepped) = state
        .history
        .lock()
        .map_err(|_| failed("history lock poisoned"))?
        .back_from(current, n)
        .map_err(bad_request)?;
    let credential = {
        let seats = state.seats.lock().await;
        table_credential(&seats)?
    };
    let what = format!("back {n}");
    let (revision, before, after) = owner_jump(
        &state,
        credential,
        commands::DevJob::Restore {
            moment: Box::new(moment),
            what: what.clone(),
        },
    )
    .await?;
    if let Ok(mut guarded) = state.history.lock() {
        guarded.set_redo(stepped, revision);
    }
    notices.push(format!("stepped {what}"));
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "restored",
            "back": n,
            "revision": {"epoch": revision.0, "sequence": revision.1},
            "before": before,
            "after": after,
            "notices": notices,
        })),
    ))
}

/// `POST /dev/forward {"n": 1}`: re-applies stepped-back states while they are
/// still valid (no fork since). Each step commits the next redo state.
pub(super) async fn step_forward(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if jumps_off() {
        return jumps_disabled();
    }
    let n = body.get("n").and_then(Value::as_u64).unwrap_or(1);
    if n == 0 || n > 64 {
        return Err(bad_request("forward needs {\"n\": 1..=64}"));
    }
    let (current, _) = current_revision(&state).await?;
    let mut notices = take_stale_notices(&state, current);
    let credential = {
        let seats = state.seats.lock().await;
        table_credential(&seats)?
    };
    let mut applied = 0u64;
    let mut revision = current;
    let mut before = String::new();
    let mut after = String::new();
    for _ in 0..n {
        let popped = state
            .history
            .lock()
            .map_err(|_| failed("history lock poisoned"))?
            .forward_pop();
        let Some(moment) = popped else {
            notices.push(format!(
                "only {applied} redo steps are left; the rest were dropped or never stepped over"
            ));
            return Err((
                axum::http::StatusCode::OK,
                Json(json!({
                    "status": "unavailable",
                    "reason": format!("only {applied} redo steps are left; the rest were dropped or never stepped over"),
                    "notices": notices,
                })),
            ));
        };
        // The popped moment is cloned for the attempt: a failed step returns
        // its entry to the redo stack instead of silently eating history.
        let step = owner_jump(
            &state,
            credential,
            commands::DevJob::Restore {
                moment: Box::new(moment.clone()),
                what: format!("forward {}", applied + 1),
            },
        )
        .await
        .inspect_err(|_| {
            if let Ok(mut guarded) = state.history.lock() {
                guarded.push_redo(moment);
            }
        })?;
        revision = step.0;
        before = step.1;
        after = step.2;
        applied += 1;
    }
    // Re-anchor the leftover redo stack at the final revision without losing it.
    if let Ok(mut guarded) = state.history.lock() {
        guarded.reanchor(revision);
    }
    notices.push(format!("stepped forward {applied}"));
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "restored",
            "forward": applied,
            "revision": {"epoch": revision.0, "sequence": revision.1},
            "before": before,
            "after": after,
            "notices": notices,
        })),
    ))
}

/// `POST /dev/override {"kind": "hp"|"budget", ...}`: a validated development
/// override committed by the owner, logged with before and after, and marked
/// edited. Shape-invalid values are refused before anything commits.
pub(super) async fn apply_override(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if overrides_off() {
        return jumps_disabled();
    }
    let kind = body
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("override needs {\"kind\": \"hp\"|\"budget\", ...}"))?;
    let (current, _) = current_revision(&state).await?;
    let mut notices = take_stale_notices(&state, current);
    let credential = {
        let seats = state.seats.lock().await;
        table_credential(&seats)?
    };
    let job = match kind {
        "hp" => {
            let hero = body.get("hero").and_then(Value::as_str).ok_or_else(|| {
                bad_request("hp override needs {\"hero\": \"<name>\", \"value\": <n>}")
            })?;
            let value = body.get("value").and_then(Value::as_i64).ok_or_else(|| {
                bad_request("hp override needs {\"hero\": \"<name>\", \"value\": <n>}")
            })?;
            let member = {
                let seats = state.seats.lock().await;
                let index = super::resolve_hero(&seats, hero).map_err(bad_request)?;
                seats.seats[index]
                    .member
                    .clone()
                    .ok_or_else(|| bad_request(format!("seat{} has not joined yet", index + 1)))?
            };
            let member: [u8; 16] = member
                .try_into()
                .map_err(|_| bad_request("hero member binding invalid; rejoin the seat"))?;
            commands::DevJob::HitPoints { member, value }
        }
        "budget" => {
            let value = body
                .get("value")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad_request("budget override needs {\"value\": <n>}"))?;
            let cap = super::super::pack::narrative_budget();
            if value > cap {
                return Err(bad_request(format!("budget must be 0..={cap} (pack cap)")));
            }
            commands::DevJob::Budget { value, cap }
        }
        _ => {
            return Err(bad_request(
                "unknown override kind; built: hp, budget. Left out and why: see the handoff",
            ));
        }
    };
    let pre = pre_jump_moment(&state);
    let (revision, before, after) = owner_jump(&state, credential, job).await?;
    let entry = json!({
        "kind": kind,
        "target": body.get("hero").cloned().unwrap_or(Value::Null),
        "before": before,
        "after": after,
        "revision": {"epoch": revision.0, "sequence": revision.1},
    });
    state.overrides.lock().await.push(entry);
    if let Ok(mut guarded) = state.history.lock() {
        guarded.set_redo(pre, revision);
    }
    notices.push("run marked edited".to_owned());
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "status": "overridden",
            "kind": kind,
            "revision": {"epoch": revision.0, "sequence": revision.1},
            "before": before,
            "after": after,
            "notices": notices,
        })),
    ))
}
