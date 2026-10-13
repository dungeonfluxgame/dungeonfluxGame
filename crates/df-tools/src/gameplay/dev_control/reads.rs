//! Read endpoints for the control surface: full state, exact per-screen views,
//! the table log, named snapshots (milestone 2 foundation) and the
//! machine-readable command spec behind `dev describe`.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::Json,
};
use df_protocol::common as rpc;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use super::{Outcome, Shared, authorized, bad_request, display_view, revision_json, seat_view};

/// `GET /dev/state`: everything a script needs in one document: room, phase,
/// scene, seats with sheets and offers, the fight, standings, revision and the
/// edited mark. Per-seat blocks carry that seat's own sheet and offers, never
/// another seat's private clue; use `view --as` to see exactly one screen.
pub(super) async fn game_state(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((axum::http::StatusCode::OK, Json(state_json(&state).await?)))
}

async fn state_json(state: &Shared) -> Result<Value, (axum::http::StatusCode, Json<Value>)> {
    let (_, display) = display_view(state).await?;
    let revision = revision_json(display.revision.as_ref());
    let journey = display_journey(&display);
    let mut seats = Vec::new();
    for index in 0..2 {
        if seat_joined(state, index).await {
            seats.push(seat_json(state, index).await?);
        }
    }
    let overrides = state.overrides.lock().await;
    let checkpoint = state.service.updates.borrow().clone();
    let history = state
        .history
        .lock()
        .map_err(|_| super::failed("history lock poisoned"))?;
    Ok(json!({
        "revision": revision,
        "room": journey.get("room_code"),
        "phase": journey.get("phase"),
        "scene": journey.get("scene"),
        "seats": seats,
        "party": journey.get("party"),
        "combat": journey.get("combat"),
        "table_choices": journey.get("table_choices"),
        "budget": checkpoint.state().narrative.remaining_budget,
        "edited": !overrides.is_empty(),
        "override_count": overrides.len(),
        "redo_depth": history.redo_len(),
        "dropped_redo": history.dropped(),
        "checkpoints": history.named_list().len(),
    }))
}

async fn seat_joined(state: &Shared, index: usize) -> bool {
    state.seats.lock().await.seats[index].credential.is_some()
}

async fn seat_json(
    state: &Shared,
    index: usize,
) -> Result<Value, (axum::http::StatusCode, Json<Value>)> {
    let hero = state.seats.lock().await.seats[index].hero.clone();
    let member = state.seats.lock().await.seats[index].member.clone();
    let (_, view) = seat_view(state, index).await?;
    let Some(rpc::view_message::Audience::Player(player)) = view.audience.as_ref() else {
        return Err(super::failed("player view required"));
    };
    let journey = player.journey.as_ref();
    // What this hero has been told (the voiced keys `social.json` keeps, per
    // NPC): the leads notes surface joins this list later.
    let told = member
        .as_deref()
        .and_then(|bytes| df_types::MemberId::from_bytes(bytes).ok())
        .map(|member| state.social.voiced_for(member))
        .unwrap_or_default();
    Ok(json!({
        "seat": index + 1,
        "hero": hero,
        "offer_id": player.offer_id,
        "action_available": player.action_available,
        "offers": player.offers.iter().map(|offer| json!({
            "action": action_name(offer.action_kind),
            "label": offer.label,
        })).collect::<Vec<_>>(),
        "sheet": player.journey.as_ref().and_then(|journey| journey.own_character.as_ref()).map(super::sheet_json),
        "standings": journey.map(|journey| journey.standing.iter().map(|standing| json!({
            "npc": standing.name,
            "attitude": attitude_name(standing.attitude),
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "told": told.iter().map(|(npc, keys)| json!({"npc": npc, "voiced": keys})).collect::<Vec<_>>(),
    }))
}

fn display_journey(view: &rpc::ViewMessage) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let Some(rpc::view_message::Audience::Display(display)) = view.audience.as_ref() else {
        return out;
    };
    if let Some(journey) = display.journey.as_ref() {
        out.insert("room_code".to_owned(), json!(journey.room_code));
        out.insert(
            "phase".to_owned(),
            json!(super::phase_name(phase_of(journey))),
        );
        out.insert(
            "scene".to_owned(),
            display
                .scene
                .as_ref()
                .map(scene_json)
                .unwrap_or(json!(null)),
        );
        out.insert(
            "party".to_owned(),
            json!(journey.party.iter().map(party_json).collect::<Vec<_>>()),
        );
        out.insert(
            "combat".to_owned(),
            journey
                .combat
                .as_ref()
                .map(combat_json)
                .unwrap_or(json!(null)),
        );
        out.insert("table_choices".to_owned(), json!(journey.table_choices));
    }
    out
}

fn phase_of(journey: &rpc::JourneyView) -> i32 {
    journey.phase
}

fn scene_json(scene: &rpc::GameplayScene) -> Value {
    json!({"title": scene.title, "place": scene.place, "rules_note": scene.rules_note})
}

fn party_json(member: &rpc::PartyMemberView) -> Value {
    json!({
        "name": member.name,
        "ready": member.character_ready,
        "hit_points": member.hit_points,
        "maximum_hit_points": member.maximum_hit_points,
        "active_turn": member.active_turn,
        "unconscious": member.unconscious,
        "cell": member.cell.as_ref().map(cell_name).unwrap_or_default(),
        "side": member.side,
        "conditions": member.conditions,
    })
}

fn cell_name(cell: &rpc::GridCell) -> String {
    format!("c{}r{}", cell.col + 1, cell.row + 1)
}

fn combat_json(combat: &rpc::CombatView) -> Value {
    let names: HashMap<Vec<u8>, String> = combat
        .participants
        .iter()
        .map(|member| (member.participant_id.clone(), member.name.clone()))
        .collect();
    json!({
        "round": combat.round,
        "active_actor": combat.active_actor,
        "finished": combat.finished,
        "layout": combat.layout_id,
        "initiative": combat.initiative.iter().map(|roll| json!({
            "name": roll.name, "total": roll.total, "order": roll.order,
        })).collect::<Vec<_>>(),
        "participants": combat.participants.iter().map(party_json).collect::<Vec<_>>(),
        "reachable_cells": combat.reachable_cells.iter().map(cell_name).collect::<Vec<_>>(),
        "legal_targets": combat.legal_targets.iter().map(|target| json!({
            "target": names.get(&target.participant_id).cloned().unwrap_or_default(),
            "weapon": target.weapon,
            "option_id": target.option_id,
            "cover": target.cover,
        })).collect::<Vec<_>>(),
        "death_saves": combat.death_saves.iter().map(|save| json!({
            "name": save.name, "successes": save.successes, "failures": save.failures,
            "stable": save.stable, "dead": save.dead,
        })).collect::<Vec<_>>(),
    })
}

fn attitude_name(attitude: u32) -> &'static str {
    match attitude {
        1 => "hostile",
        2 => "indifferent",
        3 => "friendly",
        _ => "unknown",
    }
}

fn action_name(kind: i32) -> &'static str {
    match rpc::GameplayActionKind::try_from(kind) {
        Ok(rpc::GameplayActionKind::CreateCharacter) => "create",
        Ok(rpc::GameplayActionKind::BeginStory) => "begin",
        Ok(rpc::GameplayActionKind::AskCourier) => "ask",
        Ok(rpc::GameplayActionKind::EscortCourier) => "escort",
        Ok(rpc::GameplayActionKind::DefendCourier) => "defend",
        Ok(rpc::GameplayActionKind::Move) => "move",
        Ok(rpc::GameplayActionKind::GreatswordAttack) => "attack",
        Ok(rpc::GameplayActionKind::SecondWind) => "second-wind",
        Ok(rpc::GameplayActionKind::EndTurn) => "end-turn",
        Ok(rpc::GameplayActionKind::Dash) => "dash",
        Ok(rpc::GameplayActionKind::Dodge) => "dodge",
        Ok(rpc::GameplayActionKind::Disengage) => "disengage",
        Ok(rpc::GameplayActionKind::Stand) => "stand",
        Ok(rpc::GameplayActionKind::PickUpWeapon) => "pick-up",
        Ok(rpc::GameplayActionKind::ShortRest) => "short-rest",
        Ok(rpc::GameplayActionKind::ChooseHarborScene) => "inn",
        Ok(rpc::GameplayActionKind::ResolveErrand) => "resolve",
        _ => "unknown",
    }
}

/// `GET /dev/view?as=tv|Brynn`: exactly the view that screen is being sent, so
/// private information stays visibly private.
pub(super) async fn view_as(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let who = params
        .get("as")
        .map(String::as_str)
        .ok_or_else(|| bad_request("view needs ?as=tv|<hero>"))?;
    if who.eq_ignore_ascii_case("tv") || who.eq_ignore_ascii_case("display") {
        let (revision, view) = display_view(&state).await?;
        return Ok((
            axum::http::StatusCode::OK,
            Json(
                json!({"as": "tv", "revision": revision_json(Some(&revision)), "view": view_json(&view)}),
            ),
        ));
    }
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, who).map_err(bad_request)?
    };
    let (revision, view) = seat_view(&state, index).await?;
    Ok((
        axum::http::StatusCode::OK,
        Json(
            json!({"as": who, "revision": revision_json(Some(&revision)), "view": view_json(&view)}),
        ),
    ))
}

fn view_json(view: &rpc::ViewMessage) -> Value {
    match view.audience.as_ref() {
        Some(rpc::view_message::Audience::Player(player)) => json!({
            "audience": "player",
            "offer_id": player.offer_id,
            "narration": player.narration,
            "private_clue": player.private_clue,
            "action_available": player.action_available,
            "offers": player.offers.iter().map(|offer| json!({
                "offer_id": offer.offer_id,
                "action": action_name(offer.action_kind),
                "label": offer.label,
                "destinations": offer.destinations.iter().map(|destination| destination.label.clone()).collect::<Vec<_>>(),
                "input_groups": offer.input_groups.iter().map(|group| json!({
                    "group_id": group.group_id,
                    "label": group.label,
                    "options": group.options.iter().map(|option| json!({
                        "option_id": option.option_id, "label": option.label,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "scene": player.scene.as_ref().map(scene_json).unwrap_or(json!(null)),
            "journey": player.journey.as_ref().map(journey_json).unwrap_or(json!(null)),
        }),
        Some(rpc::view_message::Audience::Display(display)) => json!({
            "audience": "display",
            "narration": display.narration,
            "scene": display.scene.as_ref().map(scene_json).unwrap_or(json!(null)),
            "journey": display.journey.as_ref().map(journey_json).unwrap_or(json!(null)),
        }),
        None => json!({"audience": "none"}),
    }
}

fn journey_json(journey: &rpc::JourneyView) -> Value {
    json!({
        "phase": super::phase_name(journey.phase),
        "room_code": journey.room_code,
        "party": journey.party.iter().map(party_json).collect::<Vec<_>>(),
        "own_character": journey.own_character.as_ref().map(super::sheet_json).unwrap_or(json!(null)),
        "creation": journey.creation.as_ref().map(|creation| json!({
            "description": creation.description,
            "groups": creation.groups.iter().map(|group| group.group_id.clone()).collect::<Vec<_>>(),
        })).unwrap_or(json!(null)),
        "combat": journey.combat.as_ref().map(combat_json).unwrap_or(json!(null)),
        "speaker": journey.speaker,
        "dialogue": journey.dialogue,
        "table_log": journey.table_log.iter().map(line_json).collect::<Vec<_>>(),
        "table_choices": journey.table_choices,
        "standing": journey.standing.iter().map(|standing| json!({
            "npc": standing.name, "attitude": attitude_name(standing.attitude),
        })).collect::<Vec<_>>(),
        "adventure_ended": journey.adventure_ended,
    })
}

fn line_json(line: &rpc::TableLine) -> Value {
    json!({
        "sequence": line.sequence,
        "speaker": line.speaker,
        "role": table_role_name(line.role),
        "kind": table_kind_name(line.kind),
        "text": line.text,
    })
}

fn table_role_name(role: i32) -> &'static str {
    match rpc::TableRole::try_from(role) {
        Ok(rpc::TableRole::Hero) => "hero",
        Ok(rpc::TableRole::Npc) => "npc",
        Ok(rpc::TableRole::Narrator) => "narrator",
        _ => "unknown",
    }
}

fn table_kind_name(kind: i32) -> &'static str {
    match rpc::TableKind::try_from(kind) {
        Ok(rpc::TableKind::Action) => "action",
        Ok(rpc::TableKind::Speech) => "speech",
        _ => "unknown",
    }
}

/// `GET /dev/log?since=<sequence>`: the table log and the override log.
/// Table lines carry a sequence, not a revision; `since` filters on it and
/// the current revision is reported alongside.
pub(super) async fn game_log(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let since: u64 = params
        .get("since")
        .map(|value| value.parse())
        .transpose()
        .map_err(|_| bad_request("since must be a table sequence number"))?
        .unwrap_or(0);
    // The owner attaches the public feed to every view it serves, so the log
    // here is exactly what the screens show; there is no separate log store.
    let (revision, view) = super::display_view(&state).await?;
    let journey = match view.audience.as_ref() {
        Some(rpc::view_message::Audience::Display(display)) => display.journey.clone(),
        _ => None,
    };
    let lines: Vec<rpc::TableLine> = journey
        .map(|journey| {
            journey
                .table_log
                .into_iter()
                .filter(|line| line.sequence > since)
                .collect()
        })
        .unwrap_or_default();
    let revision = super::revision_json(Some(&revision));
    let overrides = state.overrides.lock().await.clone();
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "revision": revision,
            "lines": lines.iter().map(line_json).collect::<Vec<_>>(),
            "overrides": overrides,
        })),
    ))
}

/// `POST /dev/checkpoint {"name": "<name>"}`: names the owner's current point
/// in the session. The stored checkpoint restores with `dev restore`.
pub(super) async fn save_checkpoint(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("checkpoint save needs {\"name\": \"<name>\"}"))?;
    if !valid_snapshot_name(name) {
        return Err(bad_request(
            "checkpoint name must be 1 to 32 lowercase letters, digits or dashes",
        ));
    }
    // The updates channel always holds the owner's current checkpoint, even
    // before this boot's first commit, so saving works right after a resume.
    // The checkpoint travels with the actor-side state it does not carry; the
    // basis is re-read after the aux snapshot so a commit landing in between
    // is retried instead of saved as a mixed moment.
    let mut moment = None;
    for _ in 0..3 {
        let checkpoint = state.service.updates.borrow().clone();
        let table_floor = state.table.position();
        let social = state.social.snapshot();
        if state.service.updates.borrow().basis().revision == checkpoint.basis().revision {
            moment = Some(super::history::SavedMoment {
                checkpoint,
                table_floor,
                social,
            });
            break;
        }
    }
    let moment = moment.unwrap_or_else(|| {
        let checkpoint = state.service.updates.borrow().clone();
        super::history::SavedMoment {
            checkpoint,
            table_floor: state.table.position(),
            social: state.social.snapshot(),
        }
    });
    let revision = checkpoint_revision_json(&moment.checkpoint);
    state
        .history
        .lock()
        .map_err(|_| super::failed("history lock poisoned"))?
        .save_named(name.to_owned(), moment)
        .map_err(bad_request)?;
    let saved = state_json(&state).await?;
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"status": "saved", "name": name, "revision": revision, "state": saved})),
    ))
}

/// The owner's checkpoint basis as JSON, without a view round trip.
pub(super) fn checkpoint_revision_json(checkpoint: &df_model::checkpoint::Checkpoint) -> Value {
    let revision = checkpoint.basis().revision;
    json!({"epoch": revision.epoch().get(), "sequence": revision.sequence()})
}

fn valid_snapshot_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `GET /dev/checkpoints`: the named points saved so far.
pub(super) async fn list_checkpoints(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let listed = state
        .history
        .lock()
        .map_err(|_| super::failed("history lock poisoned"))?
        .named_list();
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "checkpoints": listed.iter().map(|(name, revision)| json!({
                "name": name,
                "revision": {"epoch": revision.0, "sequence": revision.1},
            })).collect::<Vec<_>>(),
        })),
    ))
}

/// `GET /dev/describe`: every command, its arguments and the shape of its
/// output, so an agent can discover the tool without reading source.
pub(super) async fn describe(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    Ok((axum::http::StatusCode::OK, Json(describe_json())))
}

fn describe_json() -> Value {
    json!({
        "auth": "every request carries Authorization: Bearer <token from the run's dev-control-token file>",
        "envelope": "domain outcomes (accepted/refused/unavailable/ambiguous/said/saved) arrive with HTTP 200; malformed dev requests are HTTP 400 JSON {error}; server failures are HTTP 502/500 JSON {error}",
        "refusals": "a refused action reports the server's own reason code: stale-offer, wrong-turn, invalid-selection, paused, resource-missing, capability-unavailable, recovery-required, revision-conflict, operation-conflict, operation-expired",
        "commands": [
            {"command": "join", "method": "POST", "path": "/dev/join",
             "args": {"seat": "1 or 2"},
             "output": "{status: joined|refused, seat, receipt?}"},
            {"command": "reclaim", "method": "POST", "path": "/dev/reclaim",
             "args": {"seat": "1 or 2"},
             "output": "{status: joined|refused, seat?, receipt?, reason?}; replaces an away hero's phone after 30 seconds"},
            {"command": "create", "method": "POST", "path": "/dev/create",
             "args": {"seat": "1 or 2", "name": "hero name, 1-48 chars"},
             "output": "{status: accepted|refused|unavailable, ...}; the canonical Dwarf Fighter/Soldier is created"},
            {"command": "act", "method": "POST", "path": "/dev/act",
             "args": {"hero": "seat or hero name", "action": "begin|ask|escort|defend|move|attack|second-wind|end-turn|dash|dodge|disengage|stand|pick-up|short-rest|inn|resolve",
                     "cell": "c5r6 for move", "target": "foe or hero name for attack", "weapon": "optional weapon name", "option": "explicit option id or label"},
             "output": "{status: accepted|refused|unavailable|ambiguous|observation, revision, ...}; illegal selections are submitted so the server's refusal is reported"},
            {"command": "say", "method": "POST", "path": "/dev/say",
             "args": {"hero": "seat or hero name", "text": "1-4096 chars", "context": "question (default)|social|plan|meta|joke"},
             "output": "{status: said, disposition, npc_reply, confirmation_required, revision}"},
            {"command": "direct plate <text> [--keep] [--wait]", "method": "POST", "path": "/dev/direct",
             "args": {"kind": "plate|clear", "text": "1-1000 characters", "keep": "false by default"},
             "output": "{kind:plate, scene, key, state:pending|ready|failed, reason?:closed image failure}; --wait uses --timeout (60 seconds default, max 600)"},
            {"command": "direct clear", "method": "POST", "path": "/dev/direct",
             "args": {"kind": "clear"}, "output": "{status:cleared}"},
            {"command": "state", "method": "GET", "path": "/dev/state",
             "args": {},
             "output": "{revision, room, phase, scene, seats[{seat, hero, offers, sheet, standings, told[{npc, voiced}]}], party, combat{round, active_actor, participants, reachable_cells, legal_targets}, edited}"},
            {"command": "view", "method": "GET", "path": "/dev/view?as=tv|<hero>",
             "args": {"as": "tv or hero name"},
             "output": "{as, revision, view}: exactly the view that screen is being sent"},
            {"command": "log", "method": "GET", "path": "/dev/log?since=<sequence>",
             "args": {"since": "optional table sequence number"},
             "output": "{revision, lines[{sequence, speaker, role, kind, text}], overrides}"},
            {"command": "checkpoint-save", "method": "POST", "path": "/dev/checkpoint",
             "args": {"name": "1-32 lowercase letters, digits or dashes"},
             "output": "{status: saved, name, revision, state}; restores with dev restore"},
            {"command": "checkpoint-list", "method": "GET", "path": "/dev/checkpoints",
             "args": {},
             "output": "{checkpoints[{name, revision}]}"},
            {"command": "health", "method": "GET", "path": "/dev/health",
             "args": {},
             "output": "{status: ok, revision}; server liveness before any seat joins"},
            {"command": "restore", "method": "POST", "path": "/dev/restore",
             "args": {"name": "saved checkpoint name"},
             "output": "{status: restored, name, revision, before, after, notices}: the saved moment's full state re-committed as a new revision; playing on forks and drops the redo stack, reported in notices"},
            {"command": "back", "method": "POST", "path": "/dev/back",
             "args": {"n": "decisions to step back, default 1, at most 64 kept"},
             "output": "{status: restored, back, revision, before, after, notices}: stepped-over states become the redo stack"},
            {"command": "forward", "method": "POST", "path": "/dev/forward",
             "args": {"n": "redo steps, default 1"},
             "output": "{status: restored, forward, revision, before, after, notices}: re-applies stepped-over states while still valid"},
            {"command": "override", "method": "POST", "path": "/dev/override",
             "args": {"kind": "hp (needs hero + value) | budget (needs value)", "hero": "seat or hero name for hp", "value": "hp within the hero's bounds, budget within the pack cap"},
             "output": "DISABLED (status unavailable): direct edits need the milestone 3 projection-safety proof. Left out and why: condition, position, scene, fight, attitude, trust, dice, items (see the handoff)"},
            {"command": "wait", "client": true,
             "args": "wait --until turn <hero>|fight-end|phase <phase>|revision <epoch:seq> [--timeout <seconds>]",
             "output": "polls state until the condition holds; client-side, no server call beyond state"},
            {"command": "script", "client": true,
             "args": "script <file>: one dev command per line, stops at the first failure",
             "output": "the failing command's document; client-side"},
            {"command": "shutdown", "method": "POST", "path": "/dev/shutdown",
             "args": {},
             "output": "{status: shutting-down}; the server releases its owner lease so the database resumes immediately"},
        ],
    })
}

#[cfg(test)]
mod reclaim_tests {
    #[test]
    fn describe_advertises_reclaim_for_an_existing_seat() {
        let description = super::describe_json();
        let reclaim = description["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["command"] == "reclaim")
            .unwrap();
        assert_eq!(reclaim["method"], "POST");
        assert_eq!(reclaim["path"], "/dev/reclaim");
        assert_eq!(reclaim["args"]["seat"], "1 or 2");
    }
}
