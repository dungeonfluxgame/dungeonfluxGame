//! Read endpoints for the control surface: full state, exact per-screen views,
//! the table log, named snapshots (milestone 2 foundation) and the
//! machine-readable command spec behind `dev describe`.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use df_protocol::common as rpc;
use serde_json::{Map, Value, json};
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
        if let Some(seat) = seat_json(state, index).await? {
            seats.push(seat);
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

async fn seat_json(
    state: &Shared,
    index: usize,
) -> Result<Option<Value>, (axum::http::StatusCode, Json<Value>)> {
    let (hero, member) = {
        let seats = state.seats.lock().await;
        (
            seats.seats[index].hero.clone(),
            seats.seats[index].member.clone(),
        )
    };
    if member.is_none() {
        return Ok(None);
    }
    let (_, view) = match seat_view(state, index).await {
        Ok(view) => view,
        Err(error) if super::is_stale_seat_binding(&error) => {
            return Ok(Some(stale_seat_json(index, hero.as_deref(), &error.1.0)));
        }
        Err(error) => return Err(error),
    };
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
    Ok(Some(json!({
        "seat": index + 1,
        "hero": hero,
        "binding_status": "active",
        "offer_id": player.offer_id,
        "action_available": player.action_available,
        "offers": player.offers.iter().map(|offer| json!({
            "offer_id": offer.offer_id,
            "action": action_name(offer.action_kind),
            "label": offer.label,
        })).collect::<Vec<_>>(),
        "sheet": player.journey.as_ref().and_then(|journey| journey.own_character.as_ref()).map(character_sheet_json),
        "standings": journey.map(|journey| journey.standing.iter().map(|standing| json!({
            "npc": standing.name,
            "attitude": attitude_name(standing.attitude),
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "told": told.iter().map(|(npc, keys)| json!({"npc": npc, "voiced": keys})).collect::<Vec<_>>(),
    })))
}

fn stale_seat_json(index: usize, hero: Option<&str>, error: &Value) -> Value {
    json!({
        "seat": index + 1,
        "hero": hero,
        "binding_status": "rejoin_required",
        "rejoin_command": error.get("rejoin_command"),
        "credential_cache": error.get("credential_cache"),
        "message": error.get("message"),
    })
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

fn visual_asset_summary(scene: &rpc::GameplayScene) -> Value {
    let state = rpc::PlateState::try_from(scene.plate_state)
        .map_or("unrecognized", |state| state.as_str_name());
    let requested_source = match rpc::PlateState::try_from(scene.plate_state) {
        Ok(rpc::PlateState::Pending | rpc::PlateState::Ready | rpc::PlateState::Failed) => {
            "scene_plate"
        }
        _ => "scene_asset",
    };
    let (effective_source, effective_asset) =
        if scene.plate_state == rpc::PlateState::Ready as i32 && !scene.live_plate.is_empty() {
            ("live_plate", scene.live_plate.as_str())
        } else if !scene.live_backdrop.is_empty() {
            ("live_backdrop", scene.live_backdrop.as_str())
        } else {
            ("scene_asset", scene.scene_asset.as_str())
        };
    let asset_id =
        |asset: &str| (!asset.is_empty()).then(|| df_observe::presentation::local_identity(asset));
    json!({
        "requested_source": requested_source,
        "server_effective_source": effective_source,
        "effective_asset_id": asset_id(effective_asset),
        "plate_state": state,
        "plate_update_pending": scene.plate_update_pending,
    })
}

/// The server-derived public music plan used by the shared display. This is intentionally a
/// summary of the same pure score functions the browser consumes, so an agent can compare the
/// authoritative cue and scene family with `data-music-state` without exposing audio bytes.
fn music_plan_summary(scene: &rpc::GameplayScene, journey: &rpc::JourneyView) -> Value {
    let tempo = crate::music_score::tempo_of(scene, journey, 0);
    let facts = crate::music_score::facts_of(scene, journey, "");
    let suite = crate::music_score::suite_for(facts.in_combat, facts.outcome);
    let mix = crate::music_score::mix_for(
        tempo.phase,
        tempo.intensity,
        facts
            .outcome
            .is_some_and(|outcome| outcome == crate::music_score::Outcome::Defeat),
    );
    let colour = crate::music_score::colour_for(facts.bed, facts.in_combat);
    let layers =
        crate::music_score::scene_layer_gains(mix.layers.in_track_order(), facts.in_combat);
    let scene_profile = if facts.in_combat {
        "combat-full"
    } else {
        "social-restrained"
    };
    let plan = json!({
        "bpm": crate::music_score::BPM,
        "beat_seconds": crate::music_score::BEAT_SECONDS,
        "phase": tempo.phase.name(),
        "intensity_permille": tempo.intensity,
        "cue_sequence": tempo.cue,
        "release_hold_millis": tempo.release_hold_millis,
        "suite": suite.name(),
        "bed": facts.bed.track(),
        "scene_colour": colour.name,
        "scene_profile": scene_profile,
        "layers": {"bed": layers[0], "motif": layers[1], "tension": layers[2], "combat": layers[3]},
        "ambient_gain": mix.ambient,
        "filter_hertz": mix.filter_hertz,
        "transition_seconds": crate::music_score::TRANSITION_SECONDS,
    });
    let mut fields = Map::new();
    fields.insert(
        "resource_type".to_owned(),
        Value::String("music".to_owned()),
    );
    fields.insert(
        "resource_id".to_owned(),
        Value::String(format!("tempo:{}", tempo.cue.unwrap_or(tempo.sequence))),
    );
    fields.insert(
        "phase".to_owned(),
        Value::String(tempo.phase.name().to_owned()),
    );
    fields.insert(
        "intensity_permille".to_owned(),
        Value::from(tempo.intensity),
    );
    fields.insert("suite".to_owned(), Value::String(suite.name().to_owned()));
    fields.insert(
        "bed".to_owned(),
        Value::String(facts.bed.track().to_owned()),
    );
    fields.insert(
        "scene_colour".to_owned(),
        Value::String(colour.name.to_owned()),
    );
    fields.insert(
        "scene_profile".to_owned(),
        Value::String(scene_profile.to_owned()),
    );
    fields.insert(
        "tempo_source".to_owned(),
        Value::String(
            if tempo.cue.is_some() {
                "server_cue"
            } else {
                "view_fallback"
            }
            .to_owned(),
        ),
    );
    fields.insert("in_combat".to_owned(), Value::from(facts.in_combat));
    fields.insert("layer_gains".to_owned(), json!(layers));
    fields.insert("status".to_owned(), Value::String("projected".to_owned()));
    super::timeline::record_structured(
        "gameplay.music.plan",
        &format!(
            "MUSIC plan phase={} intensity={} suite={} bed={} colour={} profile={}",
            tempo.phase.name(),
            tempo.intensity,
            suite.name(),
            facts.bed.track(),
            colour.name,
            scene_profile
        ),
        fields,
    );
    plan
}

fn item_card_json(item: &rpc::SceneItemCard) -> Value {
    json!({
        "id": item.thing_id,
        "name": item.name,
        "detail": item.detail,
        "picture_description": item.picture_description,
        "image_path": item.image_path,
        "image_state": item.image_state,
        "status": rpc::PlateState::try_from(item.image_state)
            .map_or("unrecognized", |state| state.as_str_name()),
        "state_revision": item.state_revision,
        "observation_only": item.observation_only,
        "holder": item.holder,
        "owner_id": item.owner_id,
        "holder_id": item.holder_id,
    })
}

fn character_sheet_json(sheet: &rpc::CharacterSheet) -> Value {
    let mut projection = super::sheet_json(sheet);
    projection["owned_items"] = json!(
        sheet
            .owned_items
            .iter()
            .map(item_card_json)
            .collect::<Vec<_>>()
    );
    projection
}

fn scene_json(scene: &rpc::GameplayScene) -> Value {
    let narrative_focus = serde_json::from_str::<Value>(&scene.scene_contents_json)
        .ok()
        .and_then(|contents| contents.get("narrative_focus").cloned());
    json!({"title": scene.title, "place": scene.place, "rules_note": scene.rules_note,
        "location_id":scene.location_id, "scene_asset":scene.scene_asset, "live_backdrop":scene.live_backdrop,
        "live_plate":scene.live_plate, "plate_state":scene.plate_state,
        "plate_update_pending":scene.plate_update_pending,
        "plate_actor_heads": scene.plate_layout.as_ref().map(|layout| layout.actors.iter().map(|head| json!({"actor_id":head.actor_id,"image_path":scene.live_plate,"bubble_side":head.bubble_side,"head_x_permille":head.head_x_permille,"head_y_permille":head.head_y_permille,"head_width_permille":head.head_width_permille,"head_height_permille":head.head_height_permille})).collect::<Vec<_>>()).unwrap_or_default(),
        "plate_anchor_fallbacks": scene.cast.iter().filter(|actor| !scene.plate_layout.as_ref().is_some_and(|layout| layout.actors.iter().any(|head| head.actor_id == actor.id))).map(|actor| json!({"actor_id":actor.id,"reason":"missing-actor-head","surface":"named-lane"})).collect::<Vec<_>>(),
        "visual_assets":visual_asset_summary(scene),
        "scene_video_state":scene.scene_video_state, "scene_video_path":scene.scene_video_path,
        "cast":scene.cast.iter().map(|actor| json!({"id":actor.id,"name":actor.name,"role":actor.role,"sprite":actor.sprite,"live_sprite":actor.live_sprite,"speaking":actor.speaking})).collect::<Vec<_>>(),
        "contents": scene.scene_contents_json,
        "narrative_focus": narrative_focus,
        "item": scene.item.as_ref().map(item_card_json),
        "hand_items": scene.hand_items.iter().map(item_card_json).collect::<Vec<_>>(),
    })
}

pub(super) async fn scene_for(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let hero = params
        .get("name")
        .ok_or_else(|| bad_request("scene needs a hero name"))?;
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, hero).map_err(bad_request)?
    };
    let (_, view) = seat_view(&state, index).await?;
    let Some(rpc::view_message::Audience::Player(player)) = view.audience else {
        return Err(super::failed("player view required"));
    };
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "hero": hero, "scene": player.scene.as_ref().map(scene_json),
        })),
    ))
}

pub(super) async fn feelings(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let requested = params
        .get("name")
        .ok_or_else(|| bad_request("feelings needs an NPC id or name"))?;
    let current = state.service.updates.borrow().clone();
    let scene = crate::gameplay::beat_script_runtime::scene(current.state())
        .map_err(|_| super::failed("scene unavailable"))?;
    let id = feelings_identity(&scene, requested);
    let feelings = state
        .social
        .development_feelings(&id)
        .ok_or_else(|| bad_request("NPC profile not found"))?;
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({"npc": requested, "profile": feelings.0,
            "relationships": feelings.1.iter().map(|(hero, attitude, emotion)| json!({
                "member": hero, "attitude": format!("{attitude:?}"), "emotion": emotion,
            })).collect::<Vec<_>>(),
        })),
    ))
}

fn feelings_identity(scene: &crate::gameplay::pack::Scene, requested: &str) -> String {
    use crate::gameplay::pack::CastRole;
    let role_name = |role| match role {
        CastRole::Patron => "patron",
        CastRole::Antagonist => "antagonist",
        CastRole::Contact => "contact",
        CastRole::Bystander => "bystander",
    };
    if let Some(person) = scene.cast.iter().find(|person| {
        person.name.eq_ignore_ascii_case(requested) || person.id.eq_ignore_ascii_case(requested)
    }) {
        return format!("{}:{}", role_name(person.role), person.id);
    }
    if let Some(person) = scene
        .setting
        .as_ref()
        .and_then(|setting| setting.model.as_ref())
        .and_then(|model| {
            model.people.iter().find(|person| {
                person.name.eq_ignore_ascii_case(requested)
                    || person.id.eq_ignore_ascii_case(requested)
            })
        })
    {
        if person.is_background {
            return format!("bystander:{}", person.id);
        }
        if let Some(cast) = scene
            .cast
            .iter()
            .find(|cast| role_name(cast.role) == person.role)
        {
            return format!("{}:{}", role_name(cast.role), cast.id);
        }
    }
    requested.to_owned()
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
        "latest_sequence":combat.latest_sequence, "released_sequence":combat.released_sequence,
        "own_budget":combat.own_budget.as_ref().map(|budget|json!({"movement_ft":budget.movement_ft,"action":budget.action,"bonus_action":budget.bonus_action,"reaction":budget.reaction})),
        "own_ammunition":combat.own_ammunition.as_ref().map(|ammo|json!({"arrows":ammo.arrows,"bolts":ammo.bolts})),
        "outcomes":combat.outcomes.iter().map(super::combat_json).collect::<Vec<_>>(),
        "move_outcomes":combat.move_outcomes.iter().map(super::combat_json).collect::<Vec<_>>(),
        "death_save_outcomes":combat.death_save_outcomes.iter().map(super::combat_json).collect::<Vec<_>>(),
        "closing_lines":combat.closing_lines.iter().map(|line|json!({"speaker":line.speaker,"role":line.role,"hero_index":line.hero_index,"text":line.text})).collect::<Vec<_>>(),
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
        Ok(rpc::GameplayActionKind::StartNewAdventure) => "new-adventure",
        Ok(rpc::GameplayActionKind::ExamineHarborSeal) => "search",
        Ok(rpc::GameplayActionKind::OpportunityAttack) => "opportunity-attack",
        Ok(rpc::GameplayActionKind::DeclineReaction) => "decline-reaction",
        Ok(rpc::GameplayActionKind::StabiliseWithHealersKit) => "stabilise-kit",
        Ok(rpc::GameplayActionKind::StabiliseMedicine) => "stabilise-medicine",
        Ok(rpc::GameplayActionKind::SetTableSetting) => "table-setting",
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
                json!({"as": "tv", "revision": revision_json(Some(&revision)), "view": view_json(&view), "timings":super::timeline::snapshot(20, None, None, false)}),
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
            json!({"as": who, "revision": revision_json(Some(&revision)), "view": view_json(&view), "timings":super::timeline::snapshot(20, None, None, false)}),
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
            "check": player.check.as_ref().map(|check|check_view_json(check, player.journey.as_ref())),
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
            "journey": player.journey.as_ref().map(|journey| journey_json(journey, true)).unwrap_or(json!(null)),
        }),
        Some(rpc::view_message::Audience::Display(display)) => json!({
            "audience": "display",
            "narration": display.narration,
            "scene_graph": display.scene_graph.as_ref().map(|graph| json!({
                "revision": graph.revision, "scene_key": graph.scene_key,
                "camera": graph.camera,
                "set": graph.set.as_ref().map(|set| json!({
                    "place_id": set.place_id, "place_name": set.place_name, "asset": set.asset,
                    "exterior": set.exterior, "time_of_day": set.time_of_day, "weather": set.weather,
                    "lights": set.lights,
                    "fallback_asset": set.fallback_asset,
                })),
                "actors": graph.actors.iter().map(|actor| json!({
                    "id": actor.id, "name": actor.name, "spot": actor.spot, "pose": actor.pose,
                    "expression": actor.expression, "speaking": actor.speaking,
                    "art": actor.art, "fallback_art": actor.fallback_art, "look_at": actor.look_at,
                    "anchor_x_permille": actor.anchor_x_permille, "anchor_y_permille": actor.anchor_y_permille,
                    "depth_permille": actor.depth_permille,
                })).collect::<Vec<_>>(),
            })),
            "check": display.check.as_ref().map(|check|check_view_json(check, display.journey.as_ref())),
            "scene": display.scene.as_ref().map(scene_json).unwrap_or(json!(null)),
            "journey": display.journey.as_ref().map(|journey| journey_json(journey, false)).unwrap_or(json!(null)),
            "music_plan": display.scene.as_ref().zip(display.journey.as_ref()).map(|(scene, journey)| music_plan_summary(scene, journey)).unwrap_or(json!(null)),
        }),
        None => json!({"audience": "none"}),
    }
}

fn group_json(group: &rpc::JourneyGroup) -> Value {
    json!({"group_id": group.group_id, "label": group.label,
        "options": group.options.iter().map(|option| json!({
            "option_id": option.option_id, "label": option.label, "description": option.description,
            "requires_group": option.requires_group, "requires_options": option.requires_options,
        })).collect::<Vec<_>>()})
}

pub(super) fn identity_json(identity: &rpc::HeroIdentity) -> Value {
    json!({"appearance": identity.appearance.as_ref().map(|appearance| json!({
        "presentation":appearance.presentation, "pronouns":appearance.pronouns, "age":appearance.age,
        "build":appearance.build, "skin":appearance.skin, "hair":appearance.hair, "hair_color":appearance.hair_color,
        "eyes":appearance.eyes, "mark":appearance.mark, "attire":appearance.attire,
    })), "origin":identity.origin, "occupation":identity.occupation, "motivation":identity.motivation,
        "backstory":identity.backstory, "facts":identity.facts})
}

fn check_view_json(check: &rpc::AbilityCheckResult, journey: Option<&rpc::JourneyView>) -> Value {
    let mut value = check_json(check);
    value["actor"] = journey
        .and_then(|journey| {
            let line = journey.table_log.iter().rev().find(|line| {
                line.speaker == "Roll"
                    && line.role == rpc::TableRole::Narrator as i32
                    && line.kind == rpc::TableKind::Action as i32
            })?;
            journey
                .party
                .get(line.hero_index as usize)
                .map(|hero| json!(hero.name))
        })
        .unwrap_or(Value::Null);
    value
}

fn check_json(check: &rpc::AbilityCheckResult) -> Value {
    json!({"die":check.die, "faces":check.faces, "roll_mode":check.roll_mode,
        "ability_modifier":check.ability_modifier, "proficiency_bonus":check.proficiency_bonus,
        "total":check.total, "succeeded":check.succeeded, "source_revision":check.source_revision})
}

fn journey_json(journey: &rpc::JourneyView, include_private_character: bool) -> Value {
    json!({
        "phase": super::phase_name(journey.phase),
        "room_code": journey.room_code,
        "party": journey.party.iter().map(party_json).collect::<Vec<_>>(),
        "own_character": if include_private_character {
            journey.own_character.as_ref().map(character_sheet_json).unwrap_or(json!(null))
        } else {
            json!(null)
        },
        "creation": journey.creation.as_ref().map(|creation| json!({
            "description": creation.description,
            "source_revision": creation.source_revision,
            "groups": creation.groups.iter().map(group_json).collect::<Vec<_>>(),
            "identity_groups": creation.identity_groups.iter().map(group_json).collect::<Vec<_>>(),
            "default_identity": creation.default_identity.as_ref().map(identity_json),
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
        "notebook":journey.notebook.iter().map(|entry|json!({"kind":entry.kind,"title":entry.title,"text":entry.text,"scene":entry.scene,"order":entry.order,"owner":entry.owner,"author":entry.author,"attitude":entry.attitude})).collect::<Vec<_>>(),
        "choice_hints_hidden":journey.choice_hints_hidden,
        "presentation_sequence":journey.presentation_sequence,
        "reply_speaker":journey.reply_speaker,
        "home_video_state":journey.home_video_state,"home_video_path":journey.home_video_path,
        "tempo":journey.tempo.as_ref().map(|cue|json!({"phase":cue.phase,"intensity_permille":cue.intensity_permille,"sequence":cue.sequence,"hold_millis":cue.hold_millis})),
    })
}

fn line_json(line: &rpc::TableLine) -> Value {
    json!({
        "sequence": line.sequence,
        "actor_id": line.actor_id,
        "speaker": line.speaker,
        "hero_index": line.hero_index,
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

pub(super) async fn timeline(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let last = params
        .get("last")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| bad_request("last must be 1 to 1000"))?
        .unwrap_or(50);
    if !(1..=1000).contains(&last) {
        return Err(bad_request("last must be 1 to 1000"));
    }
    let resource_type = params.get("type").map(String::as_str);
    if resource_type.is_some_and(|value| !super::timeline::RESOURCE_TYPES.contains(&value)) {
        return Err(bad_request("type must be a known resource type"));
    }
    let component = params.get("component").map(String::as_str);
    if component.is_some_and(|value| {
        df_observe::presentation::PresentationComponent::from_label(value).is_none()
    }) {
        return Err(bad_request(
            "component must be a known presentation component",
        ));
    }
    let summary_only = params
        .get("summary")
        .map(|value| value.parse::<bool>())
        .transpose()
        .map_err(|_| bad_request("summary must be true or false"))?
        .unwrap_or(false);
    Ok((
        axum::http::StatusCode::OK,
        Json(super::timeline::snapshot(
            last,
            resource_type,
            component,
            summary_only,
        )),
    ))
}

/// `GET /dev/merchant-quotes?last=<N>`: bounded quote receipts and their
/// replay-derived outcome. The source speech text and private reply context are omitted.
pub(super) async fn merchant_quotes(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let last = params
        .get("last")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| bad_request("last must be 1 to 1000"))?
        .unwrap_or(50);
    if !(1..=1000).contains(&last) {
        return Err(bad_request("last must be 1 to 1000"));
    }

    let checkpoint = state.service.updates.borrow().clone();
    let mut history = crate::gameplay::journey::living_scene::merchant_quote_history(&checkpoint)
        .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "merchant quote history unavailable"})),
        )
    })?;
    let total = history.len();
    if history.len() > last {
        let first = history.len() - last;
        history.drain(..first);
    }
    let quotes = history
        .into_iter()
        .map(|entry| {
            let quote = entry.quote;
            let direction = match quote.direction {
                crate::gameplay::journey::living_scene::SceneTradeDirection::HeroBuys => {
                    "hero_buys"
                }
                crate::gameplay::journey::living_scene::SceneTradeDirection::HeroSells => {
                    "hero_sells"
                }
            };
            let status = match entry.status {
                crate::gameplay::journey::living_scene::MerchantQuoteStatus::Pending => "pending",
                crate::gameplay::journey::living_scene::MerchantQuoteStatus::Accepted => "accepted",
                crate::gameplay::journey::living_scene::MerchantQuoteStatus::Stale => "stale",
                crate::gameplay::journey::living_scene::MerchantQuoteStatus::Declined => "declined",
                crate::gameplay::journey::living_scene::MerchantQuoteStatus::Withdrawn => "withdrawn",
            };
            let close_reason = entry.close_reason.map(|reason| match reason {
                crate::gameplay::journey::living_scene::MerchantQuoteClosureReason::HeroDeclined => {
                    "hero_declined"
                }
                crate::gameplay::journey::living_scene::MerchantQuoteClosureReason::MerchantWithdrew => {
                    "merchant_withdrew"
                }
            });
            json!({
                "quote_id": quote.quote_id,
                "direction": direction,
                "item_id": quote.item_id,
                "offer_id": quote.offer_id,
                "seller_id": quote.seller_id,
                "buyer_id": quote.buyer_id,
                "price_cp": quote.price_cp,
                "stock_revision": quote.stock_revision,
                "purse_revision": quote.purse_revision,
                "source_line_id": quote.source_line_id,
                "source_line_sequence": quote.source_line_sequence,
                "source_revision": quote.source_revision,
                "source_actor_id": quote.source_actor_id,
                "status": status,
                "attempt_count": entry.attempt_count,
                "last_rejection": entry.last_rejection,
                "close_reason": close_reason,
            })
        })
        .collect::<Vec<_>>();
    let returned = quotes.len();
    Ok((
        StatusCode::OK,
        Json(json!({"quotes": quotes, "count": total, "returned": returned})),
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
        let notebook = state.notebook.snapshot();
        if state.service.updates.borrow().basis().revision == checkpoint.basis().revision {
            moment = Some(super::history::SavedMoment {
                checkpoint,
                table_floor,
                social,
                notebook,
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
            notebook: state.notebook.snapshot(),
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
    let actions = (1..=40)
        .filter_map(|value| rpc::GameplayActionKind::try_from(value).ok())
        .map(|kind| json!({"wire_kind":kind.as_str_name(),"action":action_name(kind as i32)}))
        .collect::<Vec<_>>();
    json!({
        "auth": "every request carries Authorization: Bearer <token from the run's dev-control-token file>",
        "envelope": "domain outcomes (accepted/refused/unavailable/ambiguous/said/saved) arrive with HTTP 200; malformed dev requests are HTTP 400 JSON {error}; a revoked local seat binding is HTTP 409 rejoin_required; server failures are HTTP 502/500 JSON {error}",
        "refusals": "a refused action reports the server's own reason code: stale-offer, wrong-turn, invalid-selection, paused, resource-missing, capability-unavailable, recovery-required, revision-conflict, operation-conflict, operation-expired",
        "phases": ["room", "creation", "opening", "dialogue", "combat", "complete", "ended (adventure_ended)"],
        "views": ["tv", "each joined seat or hero name; a revoked local binding returns an explicit rejoin_required result and never falls back to another audience"],
        "actions": actions,
        "selection": "act <hero> <offer label or id>, or act <hero> choose --option <offer label or id>; --choice group=id repeatable / --choices JSON sets every input group; uses live server offers",
        "commands": describe_commands(),
    })
}

fn describe_commands() -> Vec<Value> {
    vec![
        json!({"command":"confirm", "method":"POST", "path":"/dev/act", "args":"<hero> <action> --input-id <hex> --confirmation-token <hex> [action selections]", "output":"same admitted action receipt; requires a current pending confirmation from say"}),
        json!({"command":"cancel", "method":"POST", "path":"/dev/cancel", "args":"<hero> --input-id <hex> --confirmation-token <hex>", "output":"{status:cancelled,disposition,revision}"}),
        json!({"command":"preview", "method":"POST", "path":"/dev/preview", "args":{"seat":"seat or hero name","appearance_text":"up to 1000 characters","backstory_text":"up to 4000 characters","identity":"optional current identity JSON"}, "output":"{status:previewed, identity, source, warnings}; read-only, review then create --identity"}),
        json!({"command":"scene", "method":"GET", "path":"/dev/scene?name=<hero>", "args":{"name":"hero or seat"},"output":"current scene contents, item card and media states"}),
        json!({"command":"feelings", "method":"GET", "path":"/dev/feelings?name=<npc>", "args":{"name":"NPC id or name"},"output":"development-only profile and current relationships"}),
        json!({"command":"timeline", "method":"GET", "path":"/dev/timeline?last=<N>&type=<resource_type>&component=<component>&summary=<bool>", "args":{"last":"1..1000, default 50", "type":"text_model|speech_input|voice|image|plate|video|browser_media|state", "component":"subtitle|speech_bubble|thinking_loader|intent_loader|check_card|item_card|title_card|scene_card|plate_incoming|plate_outgoing", "summary":"true returns counts only"},"output":"{events[{sequence,unix_ms,elapsed_ms,event,line,fields}],matched,summary{window_events,resource_count,by_resource_type,by_stage,by_status,component_count,by_component,by_component_phase,by_visibility},filter,dropped,capacity,scope}; correlated lifecycle and browser delivery/source events; no prompt, transcript, credentials, raw URLs or media bytes"}),
        json!({"command":"merchant-quotes", "method":"GET", "path":"/dev/merchant-quotes?last=<N>", "args":{"last":"1..1000, default 50"}, "output":"{quotes[{quote_id,direction,item_id,offer_id,seller_id,buyer_id,price_cp,stock_revision,purse_revision,source_line_id,source_line_sequence,source_revision,source_actor_id,status,attempt_count,last_rejection,close_reason}],count,returned}; durable exact-quote outcomes and closure reasons, no source speech text or private facts"}),
        json!({"command":"layout list|get", "method":"GET", "path":"/dev/layout", "args":{}, "output":"{revision,status,overrides,visibility,clients,capacity}; actual browser IDs with per-client exact-revision rect/configured/effective-transform/visibility readback and explicit overflow"}),
        json!({"command":"layout transform <id>", "method":"POST", "path":"/dev/layout/transform", "args":"--translate-x <n> --translate-y <n> --unit px|percent [--scale <n>|--scale-x <n> --scale-y <n>] [--rotate <degrees>]", "output":"{status:pending,id,revision}; changes only the named registered visual"}),
        json!({"command":"layout move <id>", "method":"POST", "path":"/dev/layout/move", "args":"[--x <n>] [--y <n>] --unit px|percent; translation offsets compose with scale and rotation", "output":"{status:pending,id,revision}; changes bounded translation offsets"}),
        json!({"command":"layout reset [id]", "method":"POST", "path":"/dev/layout/reset", "args":"optional stable visual id; omit to reset every override", "output":"{status:pending,id,revision}; removes only owned transform rules; current game styles remain"}),
        json!({"command":"layout hide <id>", "method":"POST", "path":"/dev/layout/visibility", "args":"stable registered visual ID", "output":"{status:pending,id,visibility:hidden,revision}; hides the named component and removes pointer/accessibility exposure while preserving transforms"}),
        json!({"command":"layout show <id>", "method":"POST", "path":"/dev/layout/visibility", "args":"stable registered visual ID", "output":"{status:pending,id,visibility:visible,revision}; restores the component and its prior transform"}),
        json!({"command":"play", "client":true, "args":"--to room|creation|opening|dialogue|combat|complete|ended --timeout 600 --max-steps 256", "output":"{status:reached|play-stopped|play-timeout,target,steps,state?}; joins, creates complete heroes from the live creation offer, and follows current offers; dialogue target chooses ask and stops before combat; ordinary presentation gates remain authoritative"}),
        json!({"command": "join", "method": "POST", "path": "/dev/join", "args": {"seat": "1 or 2"}, "output": "{status: joined|refused, seat, receipt?}"}),
        json!({"command": "reclaim", "method": "POST", "path": "/dev/reclaim", "args": {"seat": "1 or 2"}, "output": "{status: joined|refused, seat?, receipt?, reason?}; explicit rejoin request that succeeds only after 30 seconds away and never displaces an active phone"}),
        json!({"command": "create", "method": "POST", "path": "/dev/create", "args": {"seat": "1 or 2", "name": "hero name, 1-48 chars", "choices":"group-id to option-id object (or --choice group=id)","identity":"optional validated identity JSON","identity_choices":"optional identity menu group-id to option-id object"}, "output": "{status: accepted|refused|unavailable, ...}; mechanical choices use server validation; a supplied identity requires canonical storage support"}),
        json!({"command": "act", "method": "POST", "path": "/dev/act", "args": {"hero": "seat or hero name", "action": "begin|ask|escort|defend|move|attack|second-wind|end-turn|dash|dodge|disengage|stand|pick-up|short-rest|inn|resolve", "cell": "c5r6 for move", "target": "foe or hero name for attack", "weapon": "optional weapon name", "option": "explicit option id or label"}, "output": "{status: accepted|refused|unavailable|ambiguous|observation, revision, ...}; illegal selections are submitted so the server's refusal is reported"}),
        json!({"command": "say", "method": "POST", "path": "/dev/say", "args": {"hero": "seat or hero name", "text": "1-4096 chars", "context": "unspecified (default; classifier/execution)|question|social|plan|meta|joke|clarify"}, "output": "{status: said, disposition, npc_reply, confirmation_required, revision}"}),
        json!({"command": "direct plate <text> [--keep] [--wait]", "method": "POST", "path": "/dev/direct", "args": {"kind": "plate|clear", "text": "1-1000 characters", "keep": "false by default"}, "output": "{kind:plate, scene, key, state:pending|ready|failed, reason?:closed image failure}; --wait uses --timeout (60 seconds default, max 600)"}),
        json!({"command": "direct clear", "method": "POST", "path": "/dev/direct", "args": {"kind": "clear"}, "output": "{status:cleared}"}),
        json!({"command": "state", "method": "GET", "path": "/dev/state", "args": {}, "output": "{revision, room, phase, scene, seats[{seat, hero, binding_status, offers, sheet, standings, told[{npc, voiced}]}], party, combat{round, active_actor, participants, reachable_cells, legal_targets}, edited}; a revoked local binding is listed as rejoin_required without private fields, while the public state remains readable"}),
        json!({"command": "view", "method": "GET", "path": "/dev/view?as=tv|<hero>", "args": {"as": "tv or hero name"}, "output": "{as, revision, view}: exactly the view that screen is being sent; a revoked local binding returns rejoin_required with an explicit dev reclaim command and no substitute audience"}),
        json!({"command": "notes", "method": "GET", "path": "/dev/notes?hero=<name>", "args": {"hero": "seat or hero name"}, "output": "{hero, entries[{kind, title, text, scene, order, owner, author}]}: private clues are filtered to that hero"}),
        json!({"command": "note", "method": "POST", "path": "/dev/note", "args": {"hero": "seat or hero name", "text": "1-4096 UTF-8 bytes"}, "output": "{status: added, hero, scene, order}: adds one shared party note"}),
        json!({"command": "log", "method": "GET", "path": "/dev/log?since=<sequence>", "args": {"since": "optional table sequence number"}, "output": "{revision, lines[{sequence, speaker, role, kind, text}], overrides}"}),
        json!({"command": "checkpoint-save", "method": "POST", "path": "/dev/checkpoint", "args": {"name": "1-32 lowercase letters, digits or dashes"}, "output": "{status: saved, name, revision, state}; restores with dev restore"}),
        json!({"command": "checkpoint-list", "method": "GET", "path": "/dev/checkpoints", "args": {}, "output": "{checkpoints[{name, revision}]}"}),
        json!({"command": "health", "method": "GET", "path": "/dev/health", "args": {}, "output": "{status: ok, revision}; server liveness before any seat joins"}),
        json!({"command": "restore", "method": "POST", "path": "/dev/restore", "args": {"name": "saved checkpoint name"}, "output": "{status: restored, name, revision, before, after, notices}: the saved moment's full state re-committed as a new revision; playing on forks and drops the redo stack, reported in notices"}),
        json!({"command": "back", "method": "POST", "path": "/dev/back", "args": {"n": "decisions to step back, default 1, at most 64 kept"}, "output": "{status: restored, back, revision, before, after, notices}: stepped-over states become the redo stack"}),
        json!({"command": "forward", "method": "POST", "path": "/dev/forward", "args": {"n": "redo steps, default 1"}, "output": "{status: restored, forward, revision, before, after, notices}: re-applies stepped-over states while still valid"}),
        json!({"command": "override", "method": "POST", "path": "/dev/override", "args": {"kind": "hp (needs hero + value) | budget (needs value)", "hero": "seat or hero name for hp", "value": "hp within the hero's bounds, budget within the pack cap"}, "output": "DISABLED (status unavailable): direct edits need the milestone 3 projection-safety proof. Left out and why: condition, position, scene, fight, attitude, trust, dice, items (see the handoff)"}),
        json!({"command": "wait", "client": true, "args": "wait --until turn <hero>|fight-end|phase <phase>|revision <epoch:seq> [--timeout <seconds>]", "output": "polls state until the condition holds; client-side, no server call beyond state"}),
        json!({"command": "script", "client": true, "args": "script <file>: one dev command per line, stops at the first failure", "output": "the failing command's document; client-side"}),
        json!({"command": "shutdown", "method": "POST", "path": "/dev/shutdown", "args": {}, "output": "{status: shutting-down}; the server releases its owner lease so the database resumes immediately"}),
    ]
}

#[cfg(test)]
mod stale_seat_projection_tests {
    use super::*;

    #[test]
    fn state_reports_stale_seat_without_falling_back_to_private_or_display_data() {
        let error = super::super::stale_seat_binding(0, Some("Brakka"), "cleared");
        assert!(super::super::is_stale_seat_binding(&error));

        let seat = stale_seat_json(0, Some("Brakka"), &error.1.0);

        assert_eq!(seat["seat"], 1);
        assert_eq!(seat["hero"], "Brakka");
        assert_eq!(seat["binding_status"], "rejoin_required");
        assert_eq!(seat["rejoin_command"], "dev reclaim 1");
        assert_eq!(seat["credential_cache"], "cleared");
        assert!(seat.get("offers").is_none());
        assert!(seat.get("sheet").is_none());
        assert!(seat.get("private_clue").is_none());
        assert!(seat.get("audience").is_none());
    }

    #[test]
    fn non_permission_owner_failures_are_not_downgraded_to_stale_seats() {
        let error = (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(json!({"error": "gameplay authority unavailable"})),
        );

        assert!(!super::super::is_stale_seat_binding(&error));
    }
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
        assert!(
            reclaim["output"]
                .as_str()
                .is_some_and(|output| output.contains("never displaces an active phone"))
        );
    }
}

#[cfg(test)]
mod visual_asset_diagnostics_tests {
    use super::*;

    #[test]
    fn scene_snapshot_shows_plate_request_and_server_fallback_source() {
        let failed = rpc::GameplayScene {
            scene_asset: "/assets/scenes/old-art.webp".to_owned(),
            live_backdrop: "/gameplay/imagery/backdrop-key".to_owned(),
            live_plate: "/gameplay/imagery/plate-key".to_owned(),
            plate_state: rpc::PlateState::Failed as i32,
            ..Default::default()
        };
        let summary = visual_asset_summary(&failed);
        assert_eq!(summary["requested_source"], "scene_plate");
        assert_eq!(summary["server_effective_source"], "live_backdrop");
        assert_eq!(
            summary["effective_asset_id"],
            df_observe::presentation::local_identity("/gameplay/imagery/backdrop-key")
        );
        assert!(!summary.to_string().contains("/gameplay/imagery/"));
        assert_eq!(summary["plate_state"], "PLATE_STATE_FAILED");

        let ready = rpc::GameplayScene {
            plate_state: rpc::PlateState::Ready as i32,
            ..failed
        };
        let view = scene_json(&ready);
        assert_eq!(view["visual_assets"]["requested_source"], "scene_plate");
        assert_eq!(
            view["visual_assets"]["server_effective_source"],
            "live_plate"
        );
        assert_eq!(
            view["visual_assets"]["effective_asset_id"],
            df_observe::presentation::local_identity("/gameplay/imagery/plate-key")
        );
    }
}

#[cfg(test)]
mod complete_view_tests {
    use super::*;

    fn item_card(
        thing_id: &str,
        detail: &str,
        image_path: &str,
        image_state: rpc::PlateState,
        observation_only: bool,
        owner_id: &str,
        holder_id: &str,
    ) -> rpc::SceneItemCard {
        rpc::SceneItemCard {
            thing_id: thing_id.into(),
            name: "Traveler’s clothes".into(),
            detail: detail.into(),
            picture_description: "A photographic close-up of folded wool and linen clothes.".into(),
            image_path: image_path.into(),
            image_state: image_state as i32,
            state_revision: 6,
            observation_only,
            holder: "Brakka".into(),
            owner_id: owner_id.into(),
            holder_id: holder_id.into(),
        }
    }

    #[test]
    fn creation_view_includes_choices_and_identity_menus() {
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    journey: Some(rpc::JourneyView {
                        creation: Some(rpc::CharacterCreationOffer {
                            groups: vec![rpc::JourneyGroup {
                                group_id: "alignment".into(),
                                label: "Alignment".into(),
                                options: vec![rpc::JourneyOption {
                                    option_id: "neutral-good".into(),
                                    label: "Neutral Good".into(),
                                    ..Default::default()
                                }],
                            }],
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let shown = view_json(&view);
        assert_eq!(
            shown["journey"]["creation"]["groups"][0]["options"][0]["option_id"],
            "neutral-good"
        );
        assert!(
            shown["journey"]["creation"]
                .get("identity_groups")
                .is_some()
        );
    }

    #[test]
    fn player_view_projects_owned_inventory_and_authoritative_custody_metadata() {
        let owned_item = item_card(
            "item-clothes-travelers",
            "Marek sold the traveler’s clothes to Brakka.",
            "/gameplay/imagery/clothes-photo",
            rpc::PlateState::Ready,
            false,
            "entity:brakka-identity",
            "entity:brakka-identity",
        );
        let held_item = item_card(
            "folded-label",
            "Marek lent the label to Brakka for inspection.",
            "",
            rpc::PlateState::Failed,
            true,
            "npc:merchant",
            "entity:brakka-identity",
        );
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Player(
                rpc::PlayerGameplayView {
                    scene: Some(rpc::GameplayScene {
                        hand_items: vec![held_item],
                        ..Default::default()
                    }),
                    journey: Some(rpc::JourneyView {
                        own_character: Some(rpc::CharacterSheet {
                            owned_items: vec![owned_item],
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let shown = view_json(&view);
        let owned = &shown["journey"]["own_character"]["owned_items"][0];
        assert_eq!(owned["id"], "item-clothes-travelers");
        assert_eq!(owned["owner_id"], "entity:brakka-identity");
        assert_eq!(owned["holder_id"], "entity:brakka-identity");
        assert_eq!(owned["observation_only"], false);
        assert_eq!(owned["image_path"], "/gameplay/imagery/clothes-photo");
        assert_eq!(owned["status"], "PLATE_STATE_READY");

        let held = &shown["scene"]["hand_items"][0];
        assert_eq!(held["id"], "folded-label");
        assert_eq!(held["owner_id"], "npc:merchant");
        assert_eq!(held["holder_id"], "entity:brakka-identity");
        assert_eq!(held["holder"], "Brakka");
        assert_eq!(held["observation_only"], true);
        assert_eq!(held["status"], "PLATE_STATE_FAILED");
    }

    #[test]
    fn display_view_redacts_a_player_owned_inventory_even_if_present_in_its_input() {
        let private_item = item_card(
            "private-owned-item-7c93",
            "A player-only purchase that must never reach the shared display.",
            "/gameplay/imagery/private-owned-item",
            rpc::PlateState::Ready,
            false,
            "entity:private-hero",
            "entity:private-hero",
        );
        let view = rpc::ViewMessage {
            audience: Some(rpc::view_message::Audience::Display(
                rpc::DisplayGameplayView {
                    journey: Some(rpc::JourneyView {
                        own_character: Some(rpc::CharacterSheet {
                            owned_items: vec![private_item],
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };

        let shown = view_json(&view);
        assert!(shown["journey"]["own_character"].is_null());
        let serialized = shown.to_string();
        assert!(!serialized.contains("private-owned-item-7c93"));
        assert!(!serialized.contains("entity:private-hero"));
        assert!(!serialized.contains("player-only purchase"));
    }
}

#[cfg(test)]
mod command_coverage_tests {
    use super::*;
    #[test]
    fn every_concrete_wire_action_has_a_named_dev_action() {
        for value in 1..=40 {
            if let Ok(kind) = rpc::GameplayActionKind::try_from(value) {
                let name = action_name(value);
                assert_ne!(name, "unknown", "{kind:?}");
                assert_eq!(
                    super::super::parse::parse_action(name).unwrap().kind(),
                    kind
                );
            }
        }
        let description = describe_json();
        for name in [
            "scene", "feelings", "timeline", "play", "preview", "confirm", "cancel", "create",
            "act", "say", "view",
        ] {
            assert!(
                description["commands"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|command| command["command"] == name),
                "{name}"
            );
        }
    }
}

#[cfg(test)]
mod feelings_identity_tests {
    use super::*;
    #[test]
    fn feelings_uses_the_same_role_qualified_identity_as_npc_speech() {
        let adventure = crate::gameplay::npc_dossier_tests::adventure(0xa7d1f297b79d0c83);
        let scene = adventure
            .pack
            .scenes
            .iter()
            .find(|scene| !scene.cast.is_empty())
            .unwrap();
        let person = &scene.cast[0];
        assert_eq!(
            feelings_identity(scene, &person.name),
            format!("patron:{}", person.id)
        );
        assert_eq!(
            feelings_identity(scene, &person.id),
            format!("patron:{}", person.id)
        );
        let mut modeled = scene.clone();
        modeled.setting = Some(adventure.settings.scene(scene.key).clone());
        let background = modeled
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .people
            .iter()
            .find(|person| person.is_background)
            .unwrap();
        assert_eq!(
            feelings_identity(&modeled, &background.name),
            format!("bystander:{}", background.id)
        );
    }
}
