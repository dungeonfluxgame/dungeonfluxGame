//! Authenticated battle inspection and typed development orders. The session owner stages,
//! validates, commits and publishes every authoritative change; reads expose committed evidence.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use df_model::checkpoint::GameState;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{Outcome, Shared, authorized, failed, require_gameplay_overrides, timeline};
use crate::gameplay::{
    beat_script_runtime,
    journey::scene_fight::{self, Presence},
    pack::Scene,
};

fn presence_label(presence: Presence) -> &'static str {
    match presence {
        Presence::Standing => "standing",
        Presence::Down { dead: false } => "down",
        Presence::Down { dead: true } => "dead",
        Presence::Gone => "gone",
    }
}

/// The quoted value of ` key="..."` in an audit line.
fn quoted(line: &str, key: &str) -> Option<String> {
    let needle = format!(" {key}=\"");
    let rest = &line[line.find(&needle)? + needle.len()..];
    Some(rest[..rest.find('"')?].to_owned())
}

/// The comma-separated items of ` key=[...]` in an audit line; an empty list when absent.
fn bracketed(line: &str, key: &str) -> Vec<String> {
    let needle = format!(" {key}=[");
    let Some(index) = line.find(&needle) else {
        return Vec::new();
    };
    let rest = &line[index + needle.len()..];
    let Some(end) = rest.find(']') else {
        return Vec::new();
    };
    rest[..end]
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The roster a `SCENE-FIGHT aggressor=…` audit line records. Lines that name no aggressor (the
/// unavailable form) are not rosters and yield `None`.
pub(super) fn roster_of(line: &str) -> Option<Value> {
    Some(json!({
        "aggressor": quoted(line, "aggressor")?,
        "target": quoted(line, "target")?,
        "join": bracketed(line, "join"),
        "flee": bracketed(line, "flee"),
        "out": bracketed(line, "out"),
    }))
}

/// The newest roster the server recorded, with the audit sequence it came from.
fn latest_roster() -> Value {
    let audit = timeline::audit(1000, Some("SCENE-FIGHT"));
    audit["events"]
        .as_array()
        .and_then(|events| {
            events.iter().rev().find_map(|event| {
                let mut roster = roster_of(event["line"].as_str()?)?;
                roster["sequence"] = event["sequence"].clone();
                Some(roster)
            })
        })
        .unwrap_or(Value::Null)
}

/// The fight document for one checkpoint. The combat view is shown only while a fight is open.
pub(super) fn fight_document(
    game: &GameState,
    scene: &Scene,
    combat: Value,
    roster: Value,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let open = scene_fight::open(game) || crate::gameplay::journey::tactical::present(game);
    let mut people = Vec::new();
    for person in &scene.cast {
        let presence = scene_fight::presence(game, &person.id)
            .map_err(|_| failed("scene person presence unavailable"))?;
        people.push(json!({
            "id": person.id,
            "name": person.name,
            "presence": presence_label(presence),
        }));
    }
    Ok(json!({
        "open": open,
        "scene": scene.title,
        "people": people,
        "roster": roster,
        "combat": if open { combat } else { Value::Null },
        "battle_scene": crate::gameplay::journey::tactical::development_scene_document(game).map_err(|_| failed("battle scene unavailable"))?,
    }))
}

/// `GET /dev/fight`: the current scene fight, its people and the latest recorded roster.
pub(super) async fn read(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let checkpoint = state.service.updates.borrow().clone();
    let scene = beat_script_runtime::scene(checkpoint.state())
        .map_err(|_| failed("current scene unavailable"))?;
    let (_, display) = super::display_view(&state).await?;
    let combat = super::reads::display_journey(&display)
        .remove("combat")
        .unwrap_or(Value::Null);
    let document = fight_document(checkpoint.state(), &scene, combat, latest_roster())?;
    Ok((StatusCode::OK, Json(document)))
}

/// Typed development command; only the owner resolves and commits state changes.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum BattleCommand {
    Start {
        foe: String,
        count: usize,
        #[serde(default)]
        level: Option<u8>,
        #[serde(default)]
        scene: Option<String>,
        #[serde(default)]
        layout: Option<String>,
    },
    End {
        outcome: String,
    },
    Restore {
        person: String,
    },
    Add {
        foe: String,
        #[serde(default)]
        cell: Option<String>,
    },
    Remove {
        name: String,
    },
    Scene {
        scene: String,
        #[serde(default)]
        layout: Option<String>,
    },
    Edit {
        name: String,
        kind: String,
        #[serde(default)]
        value: Option<i64>,
        #[serde(default)]
        condition: Option<String>,
        #[serde(default)]
        present: Option<bool>,
        #[serde(default)]
        cell: Option<String>,
        #[serde(default)]
        resource: Option<String>,
    },
}

pub(super) async fn edit(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    Json(command): Json<BattleCommand>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    require_gameplay_overrides()?;
    super::npc::commit(&state, "battle", super::npc::NpcAct::Development(command))
        .await
        .map(|answer| (StatusCode::OK, Json(answer)))
}

pub(super) async fn initiative(State(state): State<Arc<Shared>>, headers: HeaderMap) -> Outcome {
    authorized(&headers, &state.token)?;
    let current = state.service.updates.borrow().clone();
    if current.state().encounters.is_empty() {
        return Ok((StatusCode::OK, Json(json!({"initiative": []}))));
    }
    let initiative = crate::gameplay::journey::initiative_rolls(&current)
        .map_err(|error| failed(format!("initiative unavailable: {error:?}")))?;
    Ok((
        StatusCode::OK,
        Json(
            json!({"initiative":initiative.iter().map(|roll|json!({"participant_id":crate::gameplay::journey::hex(&roll.participant_id),"name":roll.name,"die":roll.die,"modifier":roll.modifier,"total":roll.total,"order":roll.order})).collect::<Vec<_>>()}),
        ),
    ))
}

#[derive(serde::Deserialize)]
pub(super) struct LogQuery {
    #[serde(default = "log_limit")]
    last: usize,
}
fn log_limit() -> usize {
    50
}
pub(super) async fn log(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<LogQuery>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    if query.last == 0 || query.last > 1000 {
        return Err(super::bad_request("--last must be 1..=1000"));
    }
    let current = state.service.updates.borrow().clone();
    let mut records = Vec::new();
    for decision in &current.state().decisions {
        if let Ok(receipt) = crate::gameplay::journey::accepted(decision) {
            for outcome in receipt.combat {
                records.push(json!({"revision":decision.revision.sequence(),"operation":crate::gameplay::journey::hex(decision.operation.as_bytes()),"outcome":super::combat_json(&outcome)}));
            }
        }
    }
    let start = records.len().saturating_sub(query.last);
    Ok((
        StatusCode::OK,
        Json(json!({"outcomes":&records[start..],"total":records.len()})),
    ))
}

#[derive(serde::Deserialize)]
pub(super) struct OffersQuery {
    hero: String,
}
pub(super) async fn offers(
    State(state): State<Arc<Shared>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OffersQuery>,
) -> Outcome {
    authorized(&headers, &state.token)?;
    let index = {
        let seats = state.seats.lock().await;
        super::resolve_hero(&seats, &query.hero).map_err(super::bad_request)?
    };
    let (_, message) = super::seat_view(&state, index).await?;
    let Some(df_protocol::common::view_message::Audience::Player(view)) = message.audience else {
        return Err(failed("player view unavailable"));
    };
    Ok((
        StatusCode::OK,
        Json(
            json!({"hero":query.hero,"offers":view.offers.iter().map(|offer|json!({"offer_id":offer.offer_id,"action_kind":offer.action_kind,"label":offer.label,"input_groups":offer.input_groups.iter().map(|group|json!({"group_id":group.group_id,"options":group.options.iter().map(|option|json!({"option_id":option.option_id,"label":option.label})).collect::<Vec<_>>()})).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_variety_tests as native, pack,
        startup_configuration::ContentRating,
    };

    #[test]
    fn roster_is_read_from_the_announced_line_and_refused_for_the_unavailable_form() {
        let line = "SCENE-FIGHT aggressor=\"Brynn\" target=\"corin\" join=[jessa] flee=[marek] out=[] rules=\"tactical.cover-centre-line: cover is adjudicated along cell centres\"";
        let roster = roster_of(line).expect("an announced fight has a roster");
        assert_eq!(roster["aggressor"], "Brynn");
        assert_eq!(roster["target"], "corin");
        assert_eq!(roster["join"], json!(["jessa"]));
        assert_eq!(roster["flee"], json!(["marek"]));
        assert_eq!(roster["out"], json!([]));

        assert!(roster_of("SCENE-FIGHT unavailable reason=\"busy\"").is_none());
    }

    #[test]
    fn fight_read_reports_no_open_fight_and_every_scene_person_standing_before_one() {
        let generated = shop::generate(46, RulesMode::Standard, ContentRating::default()).unwrap();
        let documents = generated.documents(true).unwrap();
        let (loaded, _record) = record::reload(&documents).unwrap();
        pack::with_test_pack(loaded, || {
            let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
            let scene = beat_script_runtime::scene(opening.state()).unwrap();
            let shown =
                fight_document(opening.state(), &scene, json!({"round": 1}), Value::Null).unwrap();

            assert_eq!(shown["open"], false);
            assert!(shown["combat"].is_null(), "no combat view outside a fight");
            assert!(shown["roster"].is_null());
            let people = shown["people"].as_array().unwrap();
            assert_eq!(people.len(), scene.cast.len());
            assert!(people.iter().all(|person| person["presence"] == "standing"));
        });
    }
}
